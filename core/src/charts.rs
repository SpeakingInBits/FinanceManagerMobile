//! Aggregations behind the pie and Sankey charts.

use indexmap::IndexMap;

use crate::model::{Budget, Category, Cents, Transaction, TransactionType};

pub const UNCATEGORIZED_COLOR: &str = "#9aa0a6";
/// Budget nodes in the Sankey chart; also used for budget-transfer swatches in the list.
pub const BUDGET_NODE_COLOR: &str = "#2f6fed";
const UNCATEGORIZED: &str = "uncategorized";

#[derive(Clone, PartialEq, Debug)]
pub struct CategorySlice {
    pub category_id: String,
    pub category_name: String,
    pub total: Cents,
    pub color: String,
}

fn lookup(categories: &[Category]) -> IndexMap<&str, &Category> {
    categories.iter().map(|c| (c.id.as_str(), c)).collect()
}

/// Aggregates transactions of a given type into per-category totals, largest first.
pub fn category_breakdown(
    transactions: &[Transaction],
    categories: &[Category],
    kind: TransactionType,
) -> Vec<CategorySlice> {
    let by_id = lookup(categories);
    let mut totals: IndexMap<&str, Cents> = IndexMap::new();
    for t in transactions.iter().filter(|t| t.kind == kind) {
        *totals
            .entry(t.category_id.as_deref().unwrap_or(UNCATEGORIZED))
            .or_default() += t.amount;
    }
    let mut slices: Vec<CategorySlice> = totals
        .into_iter()
        .map(|(id, total)| {
            let category = by_id.get(id);
            CategorySlice {
                category_id: id.to_string(),
                category_name: if id == UNCATEGORIZED {
                    "Uncategorized".into()
                } else {
                    category.map_or("Unknown".into(), |c| c.name.clone())
                },
                total,
                color: category.map_or(UNCATEGORIZED_COLOR.into(), |c| c.color.clone()),
            }
        })
        .collect();
    slices.sort_by_key(|s| std::cmp::Reverse(s.total));
    slices
}

#[derive(Clone, PartialEq, Debug)]
pub struct SankeyNodeDatum {
    pub name: String,
    pub color: String,
    /// True for a budget's pass-through node, rendered distinctly from a plain category.
    pub is_budget: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SankeyLinkDatum {
    pub source: usize,
    pub target: usize,
    pub value: Cents,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct SankeyGraph {
    pub nodes: Vec<SankeyNodeDatum>,
    pub links: Vec<SankeyLinkDatum>,
}

/// Builds a cash-flow graph: income sources -> Total, Total -> budgets for fills, budget ->
/// expense categories for budget-linked spending, and Total -> expense categories for the rest.
/// Pre-v7 budget-linked income links its source straight to the budget. Withdrawals enter as
/// their own "From <budget>" source nodes flowing into Total, not as a link out of the budget's
/// pass-through node, which would form a Total -> budget -> Total cycle.
pub fn build_sankey_graph(
    transactions: &[Transaction],
    categories: &[Category],
    budgets: &[Budget],
) -> SankeyGraph {
    let income = category_breakdown(transactions, categories, TransactionType::Income);
    let expense = category_breakdown(transactions, categories, TransactionType::Expense);
    let income_index: IndexMap<&str, usize> = income
        .iter()
        .enumerate()
        .map(|(i, s)| (s.category_id.as_str(), i))
        .collect();
    let expense_index: IndexMap<&str, usize> = expense
        .iter()
        .enumerate()
        .map(|(i, s)| (s.category_id.as_str(), i))
        .collect();
    let budget_by_id: IndexMap<&str, &Budget> =
        budgets.iter().map(|b| (b.id.as_str(), b)).collect();

    let mut income_to_total: IndexMap<&str, Cents> = IndexMap::new();
    let mut income_to_budget: IndexMap<&str, IndexMap<&str, Cents>> = IndexMap::new();
    let mut total_to_budget: IndexMap<&str, Cents> = IndexMap::new();
    let mut total_to_expense: IndexMap<&str, Cents> = IndexMap::new();
    let mut budget_to_expense: IndexMap<&str, IndexMap<&str, Cents>> = IndexMap::new();
    let mut withdrawal_to_total: IndexMap<&str, Cents> = IndexMap::new();
    let mut active_budget_ids: Vec<&str> = Vec::new();

    for t in transactions {
        let key = t.category_id.as_deref().unwrap_or(UNCATEGORIZED);
        let linked = t
            .budget_id
            .as_deref()
            .and_then(|id| budget_by_id.get(id))
            .map(|b| b.id.as_str());
        let mut activate = |id| {
            if !active_budget_ids.contains(&id) {
                active_budget_ids.push(id);
            }
        };
        match (t.kind, linked) {
            (TransactionType::Withdrawal, Some(b)) => {
                *withdrawal_to_total.entry(b).or_default() += t.amount
            }
            (TransactionType::Allocation, Some(b)) => {
                activate(b);
                *total_to_budget.entry(b).or_default() += t.amount;
            }
            (TransactionType::Income, Some(b)) => {
                activate(b);
                *income_to_budget
                    .entry(key)
                    .or_default()
                    .entry(b)
                    .or_default() += t.amount;
            }
            (TransactionType::Income, None) => *income_to_total.entry(key).or_default() += t.amount,
            (TransactionType::Expense, Some(b)) => {
                activate(b);
                *budget_to_expense
                    .entry(b)
                    .or_default()
                    .entry(key)
                    .or_default() += t.amount;
            }
            (TransactionType::Expense, None) => {
                *total_to_expense.entry(key).or_default() += t.amount
            }
            (TransactionType::Withdrawal | TransactionType::Allocation, None) => {}
        }
    }

    let active_budgets: Vec<&Budget> = budgets
        .iter()
        .filter(|b| active_budget_ids.contains(&b.id.as_str()))
        .collect();
    let budget_index: IndexMap<&str, usize> = active_budgets
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.as_str(), i))
        .collect();
    let withdrawn_budgets: Vec<&Budget> = budgets
        .iter()
        .filter(|b| withdrawal_to_total.get(b.id.as_str()).copied().unwrap_or(0) > 0)
        .collect();

    let withdrawal_offset = income.len();
    let budget_offset = withdrawal_offset + withdrawn_budgets.len();
    let total_index = budget_offset + active_budgets.len();
    let expense_offset = total_index + 1;

    let mut nodes: Vec<SankeyNodeDatum> = Vec::new();
    nodes.extend(income.iter().map(|s| SankeyNodeDatum {
        name: s.category_name.clone(),
        color: s.color.clone(),
        is_budget: false,
    }));
    nodes.extend(withdrawn_budgets.iter().map(|b| SankeyNodeDatum {
        name: format!("From {}", b.name),
        color: BUDGET_NODE_COLOR.into(),
        is_budget: true,
    }));
    nodes.extend(active_budgets.iter().map(|b| SankeyNodeDatum {
        name: b.name.clone(),
        color: BUDGET_NODE_COLOR.into(),
        is_budget: true,
    }));
    nodes.push(SankeyNodeDatum {
        name: "Total".into(),
        color: UNCATEGORIZED_COLOR.into(),
        is_budget: false,
    });
    nodes.extend(expense.iter().map(|s| SankeyNodeDatum {
        name: s.category_name.clone(),
        color: s.color.clone(),
        is_budget: false,
    }));

    let mut links = Vec::new();
    let mut push = |source: usize, target: usize, value: Cents| {
        if value > 0 {
            links.push(SankeyLinkDatum {
                source,
                target,
                value,
            });
        }
    };
    for (key, value) in &income_to_total {
        push(income_index[key], total_index, *value);
    }
    for (key, by_budget) in &income_to_budget {
        for (budget_id, value) in by_budget {
            push(
                income_index[key],
                budget_offset + budget_index[budget_id],
                *value,
            );
        }
    }
    for (i, b) in withdrawn_budgets.iter().enumerate() {
        push(
            withdrawal_offset + i,
            total_index,
            withdrawal_to_total[b.id.as_str()],
        );
    }
    for (budget_id, value) in &total_to_budget {
        push(total_index, budget_offset + budget_index[budget_id], *value);
    }
    for (key, value) in &total_to_expense {
        push(total_index, expense_offset + expense_index[key], *value);
    }
    for (budget_id, by_category) in &budget_to_expense {
        for (key, value) in by_category {
            push(
                budget_offset + budget_index[budget_id],
                expense_offset + expense_index[key],
                *value,
            );
        }
    }

    SankeyGraph { nodes, links }
}

#[derive(Clone, PartialEq, Debug)]
pub struct CategorySubSlice {
    pub category_id: String,
    pub subcategory_id: Option<String>,
    pub label: String,
    pub total: Cents,
    pub color: String,
}

#[derive(Clone, PartialEq, Debug)]
pub struct CategoryGroupSlice {
    pub category_id: String,
    pub category_name: String,
    pub color: String,
    pub total: Cents,
    /// One slice per subcategory in use, plus an "Other" bucket for direct (no-subcategory)
    /// transactions when subcategories are also in use; a single slice (no split) otherwise.
    pub slices: Vec<CategorySubSlice>,
}

/// Like [`category_breakdown`], but further splits each category into its subcategories for the
/// pie chart's drilldown.
pub fn category_breakdown_by_subcategory(
    transactions: &[Transaction],
    categories: &[Category],
    kind: TransactionType,
) -> Vec<CategoryGroupSlice> {
    let by_id = lookup(categories);
    let mut by_category: IndexMap<&str, IndexMap<Option<&str>, Cents>> = IndexMap::new();
    for t in transactions.iter().filter(|t| t.kind == kind) {
        let cat_key = t.category_id.as_deref().unwrap_or(UNCATEGORIZED);
        *by_category
            .entry(cat_key)
            .or_default()
            .entry(t.subcategory_id.as_deref())
            .or_default() += t.amount;
    }

    let mut groups: Vec<CategoryGroupSlice> = by_category
        .into_iter()
        .map(|(cat_id, by_sub)| {
            let category = by_id.get(cat_id);
            let category_name = if cat_id == UNCATEGORIZED {
                "Uncategorized".to_string()
            } else {
                category.map_or("Unknown".into(), |c| c.name.clone())
            };
            let base_color = category.map_or(UNCATEGORIZED_COLOR.to_string(), |c| c.color.clone());
            let mut entries: Vec<(Option<&str>, Cents)> = by_sub.into_iter().collect();
            entries.sort_by_key(|e| std::cmp::Reverse(e.1));
            let total = entries.iter().map(|e| e.1).sum();
            let split = entries.len() > 1;
            let slices = entries
                .into_iter()
                .map(|(sub_id, total)| {
                    let sub = sub_id.and_then(|id| by_id.get(id));
                    let label = match sub_id {
                        Some(_) => sub.map_or("Unknown".into(), |c| c.name.clone()),
                        None if split => "Other".into(),
                        None => category_name.clone(),
                    };
                    CategorySubSlice {
                        category_id: cat_id.to_string(),
                        subcategory_id: sub_id.map(str::to_string),
                        label,
                        total,
                        // Each subcategory keeps its own color; "Other" and unsplit categories
                        // take the parent's.
                        color: sub.map_or_else(|| base_color.clone(), |c| c.color.clone()),
                    }
                })
                .collect();
            CategoryGroupSlice {
                category_id: cat_id.to_string(),
                category_name,
                color: base_color,
                total,
                slices,
            }
        })
        .collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.total));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recurrence::tests::tx;

    fn cat(id: &str, parent: Option<&str>, color: &str) -> Category {
        Category {
            id: id.into(),
            name: id.to_uppercase(),
            parent_id: parent.map(Into::into),
            color: color.into(),
            icon: None,
            order: None,
            created_at: 0,
        }
    }

    fn with(
        mut t: Transaction,
        cat: Option<&str>,
        sub: Option<&str>,
        budget: Option<&str>,
    ) -> Transaction {
        t.category_id = cat.map(Into::into);
        t.subcategory_id = sub.map(Into::into);
        t.budget_id = budget.map(Into::into);
        t
    }

    #[test]
    fn breakdown_by_subcategory_adds_other_bucket() {
        let cats = vec![
            cat("food", None, "#111111"),
            cat("grocery", Some("food"), "#222222"),
        ];
        let txs = vec![
            with(
                tx(TransactionType::Expense, 300, 0),
                Some("food"),
                Some("grocery"),
                None,
            ),
            with(
                tx(TransactionType::Expense, 100, 1),
                Some("food"),
                None,
                None,
            ),
            with(tx(TransactionType::Expense, 50, 2), None, None, None),
            with(
                tx(TransactionType::Income, 999, 3),
                Some("food"),
                None,
                None,
            ),
        ];
        let groups = category_breakdown_by_subcategory(&txs, &cats, TransactionType::Expense);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].total, 400);
        assert_eq!(groups[0].slices[0].label, "GROCERY");
        assert_eq!(groups[0].slices[0].color, "#222222");
        assert_eq!(groups[0].slices[1].label, "Other");
        assert_eq!(groups[0].slices[1].color, "#111111");
        assert_eq!(groups[1].category_name, "Uncategorized");
        assert_eq!(groups[1].slices[0].label, "Uncategorized");
    }

    #[test]
    fn sankey_routes_budgets_and_withdrawals() {
        let cats = vec![cat("pay", None, "#00ff00"), cat("rent", None, "#ff0000")];
        let budgets = vec![Budget {
            id: "b".into(),
            name: "Rainy day".into(),
            description: String::new(),
            target_amount: 0,
            period_type: crate::model::BudgetPeriodType::Monthly,
            start_date: 0,
            end_date: None,
            category_id: None,
            subcategory_id: None,
            created_at: 0,
        }];
        let txs = vec![
            with(
                tx(TransactionType::Income, 1000, 0),
                Some("pay"),
                None,
                None,
            ),
            with(
                tx(TransactionType::Allocation, 300, 1),
                None,
                None,
                Some("b"),
            ),
            with(
                tx(TransactionType::Expense, 200, 2),
                Some("rent"),
                None,
                Some("b"),
            ),
            with(
                tx(TransactionType::Expense, 400, 3),
                Some("rent"),
                None,
                None,
            ),
            with(
                tx(TransactionType::Withdrawal, 50, 4),
                None,
                None,
                Some("b"),
            ),
        ];
        let g = build_sankey_graph(&txs, &cats, &budgets);
        let names: Vec<&str> = g.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(
            names,
            ["PAY", "From Rainy day", "Rainy day", "Total", "RENT"]
        );
        assert_eq!(
            g.links,
            vec![
                SankeyLinkDatum {
                    source: 0,
                    target: 3,
                    value: 1000
                },
                SankeyLinkDatum {
                    source: 1,
                    target: 3,
                    value: 50
                },
                SankeyLinkDatum {
                    source: 3,
                    target: 2,
                    value: 300
                },
                SankeyLinkDatum {
                    source: 3,
                    target: 4,
                    value: 400
                },
                SankeyLinkDatum {
                    source: 2,
                    target: 4,
                    value: 200
                },
            ]
        );
    }
}
