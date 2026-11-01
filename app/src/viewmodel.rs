//! Builds the UI's view models from app data: dashboard figures, budget cards, the grouped
//! transaction list, the category tree, and chart geometry.

use std::cmp::Ordering;
use std::collections::HashSet;

use finance_core::budget::compute_budget_stats;
use finance_core::charts::{
    BUDGET_NODE_COLOR, CategoryGroupSlice, UNCATEGORIZED_COLOR, build_sankey_graph,
    category_breakdown_by_subcategory,
};
use finance_core::currency::format_cents;
use finance_core::date::format_date;
use finance_core::geometry::{PieLayout, Sankey, pie_layout};
use finance_core::model::{
    Budget, BudgetPeriodType, Category, Cents, RecurrenceFrequency, Transaction, TransactionType,
};
use finance_core::recurrence::{MonthlyOccurrence, occurrences_for_month};
use finance_core::store::sort_categories;
use indexmap::IndexMap;
use slint::{Color, ModelRc, SharedString, VecModel};

use crate::{
    BudgetCardData, CategoryRow, LegendEntry, PieData, PieSlice, SankeyLinkView, SankeyNodeView,
    Stat, TxRow,
};

/// Side of the square view box pie paths are laid out in.
pub const PIE_SIZE: f64 = 300.0;
pub const SANKEY_HEIGHT: f64 = 320.0;

pub fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// Parses "#rrggbb" (or "#rgb"); falls back to the uncategorized gray.
pub fn parse_color(hex: &str) -> Color {
    let h = hex.trim().trim_start_matches('#');
    let expanded: String = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect()
    } else {
        h.to_string()
    };
    match u32::from_str_radix(&expanded, 16) {
        Ok(v) if expanded.len() == 6 => {
            Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8)
        }
        _ => Color::from_rgb_u8(0x9a, 0xa0, 0xa6),
    }
}

pub fn color_to_hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.red(), c.green(), c.blue())
}

/// Dashboard totals for one month, mirroring the web dashboard.
pub struct MonthSummary {
    pub income: Cents,
    pub recurring_expense: Cents,
    pub one_time_expense: Cents,
    pub net: Cents,
    pub allocations: Cents,
    /// Net after allocations: the pool "Allocate remaining funds" can draw from.
    pub remaining: Cents,
    /// Unbudgeted transactions in the month (charts' input).
    pub not_budgeted: Vec<Transaction>,
}

pub fn month_summary(in_month: &[Transaction]) -> MonthSummary {
    // Money earmarked for a budget is a fill into (or spend from) that budget's own balance, not
    // part of this month's regular cash flow.
    let not_budgeted: Vec<Transaction> = in_month
        .iter()
        .filter(|t| t.budget_id.is_none())
        .cloned()
        .collect();
    let sum = |it: &mut dyn Iterator<Item = &Transaction>| it.map(|t| t.amount).sum::<Cents>();
    // Fills: allocations, plus pre-v7 budget-linked income counted the same way.
    let allocations = sum(&mut in_month.iter().filter(|t| {
        t.budget_id.is_some()
            && matches!(
                t.kind,
                TransactionType::Allocation | TransactionType::Income
            )
    }));
    // Withdrawals move money back into general income, so they count as income this month.
    let withdrawals = sum(&mut in_month
        .iter()
        .filter(|t| t.kind == TransactionType::Withdrawal));
    let income = sum(&mut not_budgeted
        .iter()
        .filter(|t| t.kind == TransactionType::Income))
        + withdrawals;
    let expense = |recurring: bool| {
        sum(&mut not_budgeted
            .iter()
            .filter(|t| t.kind == TransactionType::Expense && t.recurrence.is_some() == recurring))
    };
    let recurring_expense = expense(true);
    let one_time_expense = expense(false);
    let net = income - recurring_expense - one_time_expense;
    MonthSummary {
        income,
        recurring_expense,
        one_time_expense,
        net,
        allocations,
        remaining: net - allocations,
        not_budgeted,
    }
}

pub fn stats(s: &MonthSummary) -> Vec<Stat> {
    [
        ("Income", s.income),
        ("Recurring expenses", s.recurring_expense),
        ("One-time expenses", s.one_time_expense),
        ("Net", s.net),
        ("Contrib. to budgets", s.allocations),
        ("Net after allocations", s.remaining),
    ]
    .into_iter()
    .map(|(label, value)| Stat {
        label: label.into(),
        value: format_cents(value).into(),
    })
    .collect()
}

/// Expense transactions of the summary, optionally narrowed to recurring or one-time ones.
pub fn expenses(s: &MonthSummary, recurring: Option<bool>) -> Vec<Transaction> {
    s.not_budgeted
        .iter()
        .filter(|t| {
            t.kind == TransactionType::Expense
                && recurring.is_none_or(|r| t.recurrence.is_some() == r)
        })
        .cloned()
        .collect()
}

/// Pie view model plus the layout kept for hit-testing taps.
pub fn pie(id: i32, groups: &[CategoryGroupSlice]) -> (PieData, PieLayout) {
    let layout = pie_layout(groups, PIE_SIZE);
    let total: Cents = groups.iter().map(|g| g.total).sum();
    let pct = |v: Cents| {
        if total > 0 {
            (v as f64 / total as f64 * 100.0).round() as i64
        } else {
            0
        }
    };
    let slices = layout
        .arcs
        .iter()
        .map(|a| PieSlice {
            path: a.path.as_str().into(),
            color: parse_color(&a.color),
            heading: a.heading.as_str().into(),
            value: format!("{} ({}%)", format_cents(a.total), pct(a.total)).into(),
        })
        .collect();
    let mut legend = Vec::new();
    for g in groups {
        legend.push(LegendEntry {
            label: format!(
                "{} — {} ({}%)",
                g.category_name,
                format_cents(g.total),
                pct(g.total)
            )
            .into(),
            color: parse_color(&g.color),
            child: false,
        });
        if g.slices.len() > 1 {
            legend.extend(g.slices.iter().map(|s| {
                LegendEntry {
                    label: format!(
                        "{} — {} ({}%)",
                        s.label,
                        format_cents(s.total),
                        pct(s.total)
                    )
                    .into(),
                    color: parse_color(&s.color),
                    child: true,
                }
            }));
        }
    }
    (
        PieData {
            id,
            slices: model(slices),
            legend: model(legend),
        },
        layout,
    )
}

pub fn breakdown(
    transactions: &[Transaction],
    categories: &[Category],
    kind: TransactionType,
) -> Vec<CategoryGroupSlice> {
    category_breakdown_by_subcategory(transactions, categories, kind)
}

pub fn budget_cards(
    budgets: &[Budget],
    categories: &[Category],
    transactions: &[Transaction],
    month: i64,
) -> Vec<BudgetCardData> {
    let name_of = |id: &Option<String>| {
        id.as_deref()
            .and_then(|id| categories.iter().find(|c| c.id == id))
            .map(|c| c.name.clone())
    };
    budgets
        .iter()
        .map(|b| {
            let category = name_of(&b.category_id).unwrap_or_else(|| "General".into());
            let category = match name_of(&b.subcategory_id) {
                Some(sub) => format!("{category} · {sub}"),
                None => category,
            };
            let one_time = b.period_type == BudgetPeriodType::OneTime;
            let s = compute_budget_stats(b, transactions, month);
            let pct = (s.progress_percent * 100.0).min(100.0);
            let state = if one_time && s.overdrawn {
                2
            } else if (85.0..100.0).contains(&pct) {
                1
            } else {
                0
            };
            BudgetCardData {
                id: b.id.as_str().into(),
                name: b.name.as_str().into(),
                meta: format!(
                    "{} · {category}",
                    if one_time { "One-time" } else { "Monthly" }
                )
                .into(),
                fill: (pct / 100.0) as f32,
                state,
                amount_label: format!(
                    "{} {}",
                    format_cents(s.progress),
                    if one_time { "available" } else { "contributed" }
                )
                .into(),
                goal_label: format!("{} goal", format_cents(s.target)).into(),
                balance_label: if one_time {
                    SharedString::new()
                } else {
                    format!("Lifetime Balance: {}", format_cents(s.balance)).into()
                },
                balance_over: s.overdrawn,
            }
        })
        .collect()
}

const UNCATEGORIZED: &str = "__uncategorized__";
const NO_SUBCATEGORY: &str = "__none__";

fn name_for(key: &str, categories: &[Category]) -> String {
    match key {
        UNCATEGORIZED => "Uncategorized".into(),
        NO_SUBCATEGORY => "Other".into(),
        id => categories
            .iter()
            .find(|c| c.id == id)
            .map_or("Unknown".into(), |c| c.name.clone()),
    }
}

/// Alphabetical (case-insensitive), but the catch-all bucket (Uncategorized / Other) sorts last.
fn compare_names(a: &str, b: &str, a_last: bool, b_last: bool) -> Ordering {
    a_last
        .cmp(&b_last)
        .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
        .then_with(|| a.cmp(b))
}

/// The month's transactions, flattened into section / category / subcategory headers and items.
pub fn transaction_rows(
    transactions: &[Transaction],
    categories: &[Category],
    budgets: &[Budget],
    month: i64,
) -> Vec<TxRow> {
    let occurrences = occurrences_for_month(transactions, month);
    let mut rows = Vec::new();
    for (title, recurring) in [("Recurring", true), ("One-time", false)] {
        let group: Vec<&MonthlyOccurrence> = occurrences
            .iter()
            .filter(|o| o.transaction.recurrence.is_some() == recurring)
            .collect();
        if group.is_empty() {
            continue;
        }
        rows.push(header(0, title, Color::default()));

        let mut by_category: IndexMap<&str, Vec<&MonthlyOccurrence>> = IndexMap::new();
        for o in group
            .iter()
            .filter(|o| !o.transaction.kind.is_budget_transfer())
        {
            by_category
                .entry(
                    o.transaction
                        .category_id
                        .as_deref()
                        .unwrap_or(UNCATEGORIZED),
                )
                .or_default()
                .push(o);
        }
        let mut entries: Vec<_> = by_category.into_iter().collect();
        entries.sort_by(|(a, _), (b, _)| {
            compare_names(
                &name_for(a, categories),
                &name_for(b, categories),
                *a == UNCATEGORIZED,
                *b == UNCATEGORIZED,
            )
        });
        for (key, items) in entries {
            let color = categories
                .iter()
                .find(|c| c.id == key)
                .map_or(UNCATEGORIZED_COLOR, |c| c.color.as_str());
            rows.push(header(1, &name_for(key, categories), parse_color(color)));
            subcategory_rows(&mut rows, &items, categories, budgets);
        }

        for (title, kind) in [
            ("Budget fills", TransactionType::Allocation),
            ("Budget withdrawals", TransactionType::Withdrawal),
        ] {
            let items: Vec<&MonthlyOccurrence> = group
                .iter()
                .copied()
                .filter(|o| o.transaction.kind == kind)
                .collect();
            if !items.is_empty() {
                rows.push(header(1, title, parse_color(BUDGET_NODE_COLOR)));
                item_rows(&mut rows, &items, categories, budgets);
            }
        }
    }
    rows
}

/// Within a category, groups by subcategory. Direct (no-subcategory) transactions list straight
/// under the category when it has no real subcategories in use, otherwise under "Other".
fn subcategory_rows(
    rows: &mut Vec<TxRow>,
    items: &[&MonthlyOccurrence],
    categories: &[Category],
    budgets: &[Budget],
) {
    let mut by_sub: IndexMap<&str, Vec<&MonthlyOccurrence>> = IndexMap::new();
    for o in items {
        by_sub
            .entry(
                o.transaction
                    .subcategory_id
                    .as_deref()
                    .unwrap_or(NO_SUBCATEGORY),
            )
            .or_default()
            .push(o);
    }
    let has_real = by_sub.keys().any(|k| *k != NO_SUBCATEGORY);
    let mut entries: Vec<_> = by_sub.into_iter().collect();
    entries.sort_by(|(a, _), (b, _)| {
        compare_names(
            &name_for(a, categories),
            &name_for(b, categories),
            *a == NO_SUBCATEGORY,
            *b == NO_SUBCATEGORY,
        )
    });
    for (key, subs) in entries {
        if key != NO_SUBCATEGORY || has_real {
            rows.push(header(2, &name_for(key, categories), Color::default()));
        }
        item_rows(rows, &subs, categories, budgets);
    }
}

fn header(kind: i32, title: &str, color: Color) -> TxRow {
    TxRow {
        kind,
        title: title.into(),
        color,
        ..Default::default()
    }
}

/// Transaction items, newest first.
fn item_rows(
    rows: &mut Vec<TxRow>,
    items: &[&MonthlyOccurrence],
    categories: &[Category],
    budgets: &[Budget],
) {
    let mut sorted = items.to_vec();
    sorted.sort_by_key(|o| std::cmp::Reverse(o.display_date));
    for o in sorted {
        let t = o.transaction;
        let category = t
            .category_id
            .as_deref()
            .and_then(|id| categories.iter().find(|c| c.id == id));
        let category_name = category.map_or("Uncategorized", |c| c.name.as_str());
        let budget_name = t
            .budget_id
            .as_deref()
            .and_then(|id| budgets.iter().find(|b| b.id == id))
            .map(|b| b.name.as_str());
        let transfer_name = budget_name.unwrap_or("Budget");
        let (sign, label, meta, amount_kind) = match t.kind {
            TransactionType::Allocation => {
                ("→", transfer_name, format!("Fill: {transfer_name}"), 2)
            }
            TransactionType::Withdrawal => (
                "+",
                transfer_name,
                format!("Withdrawal: {transfer_name}"),
                0,
            ),
            TransactionType::Income => ("+", category_name, category_name.to_string(), 0),
            TransactionType::Expense => ("-", category_name, category_name.to_string(), 1),
        };
        let recurrence = match t.recurrence {
            Some(RecurrenceFrequency::Monthly) => {
                format!(" · Monthly ({}/mo)", format_cents(t.amount))
            }
            Some(RecurrenceFrequency::Yearly) => {
                format!(" · Yearly ({}/yr)", format_cents(t.amount))
            }
            None => String::new(),
        };
        // Expenses paid from a budget draw down that budget's balance; name it on its own line.
        let budget_line = if t.kind == TransactionType::Expense && t.budget_id.is_some() {
            budget_name.unwrap_or("Deleted budget")
        } else {
            ""
        };
        let color = if t.kind.is_budget_transfer() {
            BUDGET_NODE_COLOR
        } else {
            category.map_or(UNCATEGORIZED_COLOR, |c| c.color.as_str())
        };
        rows.push(TxRow {
            kind: 3,
            id: t.id.as_str().into(),
            title: if t.note.is_empty() {
                label.into()
            } else {
                t.note.as_str().into()
            },
            color: parse_color(color),
            meta: format!("{meta} · {}{recurrence}", format_date(o.display_date)).into(),
            budget_line: budget_line.into(),
            amount: format!("{sign}{}", format_cents(o.display_amount)).into(),
            amount_kind,
        });
    }
}

/// The category tree, flattened depth-first in display order, skipping collapsed subtrees.
pub fn category_rows(categories: &[Category], collapsed: &HashSet<String>) -> Vec<CategoryRow> {
    fn walk(
        parent: Option<&str>,
        depth: i32,
        categories: &[Category],
        collapsed: &HashSet<String>,
        out: &mut Vec<CategoryRow>,
    ) {
        let siblings = sort_categories(
            categories
                .iter()
                .filter(|c| c.parent_id.as_deref() == parent),
        );
        let count = siblings.len();
        for (i, c) in siblings.into_iter().enumerate() {
            let has_children = categories
                .iter()
                .any(|x| x.parent_id.as_deref() == Some(c.id.as_str()));
            let is_collapsed = collapsed.contains(&c.id);
            out.push(CategoryRow {
                id: c.id.as_str().into(),
                name: c.name.as_str().into(),
                color: parse_color(&c.color),
                depth,
                has_children,
                collapsed: is_collapsed,
                can_up: i > 0,
                can_down: i + 1 < count,
            });
            if has_children && !is_collapsed {
                walk(Some(&c.id), depth + 1, categories, collapsed, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(None, 0, categories, collapsed, &mut out);
    out
}

/// Sankey nodes and links laid out for a chart `width` logical pixels wide.
pub fn sankey(
    in_month: &[Transaction],
    categories: &[Category],
    budgets: &[Budget],
    width: f64,
) -> (Vec<SankeyNodeView>, Vec<SankeyLinkView>) {
    let graph = build_sankey_graph(in_month, categories, budgets);
    let width = width.max(200.0);
    let layout = Sankey {
        x0: 4.0,
        y0: 4.0,
        x1: width - 4.0,
        y1: SANKEY_HEIGHT - 4.0,
        node_width: 14.0,
        node_padding: 12.0,
        iterations: 6,
    }
    .layout(&graph);
    let nodes = layout
        .nodes
        .iter()
        .zip(&graph.nodes)
        .map(|(n, d)| {
            let label = if d.is_budget {
                format!("{} (budget)", d.name)
            } else {
                d.name.clone()
            };
            SankeyNodeView {
                x: n.x0 as f32,
                y: n.y0 as f32,
                w: (n.x1 - n.x0) as f32,
                h: (n.y1 - n.y0) as f32,
                color: parse_color(&d.color),
                budget: d.is_budget,
                detail: format!("{label}: {}", format_cents(n.value)).into(),
                label: label.into(),
                label_left: n.x0 >= width / 2.0,
            }
        })
        .collect();
    let links = layout
        .links
        .iter()
        .map(|l| SankeyLinkView {
            path: l.path.as_str().into(),
            color: parse_color(&graph.nodes[l.source].color),
        })
        .collect();
    (nodes, links)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_round_trip() {
        assert_eq!(color_to_hex(parse_color("#2f6fed")), "#2f6fed");
        assert_eq!(color_to_hex(parse_color("#abc")), "#aabbcc");
        assert_eq!(color_to_hex(parse_color("nope")), "#9aa0a6");
    }
}
