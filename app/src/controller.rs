//! Owns the app data and UI state, handles every UI callback, and pushes fresh view models to
//! the window after each change.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use finance_core::budget::withdrawable_budgets;
use finance_core::currency::{cents_to_input, format_cents, parse_to_cents};
use finance_core::date::{
    format_date, format_month_year, local_midnight, now, shift_month, start_of_month, ymd,
};
use finance_core::geometry::PieLayout;
use finance_core::model::{
    Budget, BudgetPeriodType, Category, Cents, Millis, NewBudget, NewCategory, NewTransaction,
    RecurrenceFrequency, ThemeMode, Transaction, TransactionType,
};
use finance_core::recurrence::{month_view, monthly_equivalent_amount};
use finance_core::store::{Direction, Store, sort_categories};
use slint::{ComponentHandle, ModelRc, SharedString, Weak};

use crate::viewmodel::{self, model, parse_color};
use crate::{AllocRow, App, AppWindow, Date, Theme};

const DEFAULT_CATEGORY_COLOR: &str = "#2f6fed";

/// What a confirm dialog deletes when accepted.
enum Pending {
    Transaction(String),
    Category(String),
    Budget(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sheet {
    Closed = 0,
    Transaction = 1,
    Budget = 2,
    Category = 3,
    Allocate = 4,
    Withdraw = 5,
}

struct State {
    store: Store,
    /// Start-of-month millis of the month being viewed.
    selected_month: Millis,
    collapsed: HashSet<String>,
    sankey_width: f64,
    /// Pie layouts by chart id, for hit-testing taps: 0 = dashboard combined, 1 = recurring,
    /// 2 = one-time, 3 = charts page.
    pies: [PieLayout; 4],
    /// Ids behind the top-level category options (index 0 = none).
    top_category_ids: Vec<Option<String>>,
    editing_transaction: Option<String>,
    editing_budget: Option<String>,
    editing_category: Option<String>,
    category_parent_ids: Vec<Option<String>>,
    /// Remaining funds when the allocate sheet was opened, and its per-budget inputs.
    alloc_available: Cents,
    alloc_amounts: Vec<String>,
    /// Withdrawable budgets when the withdraw sheet was opened.
    withdraw_options: Vec<(String, Cents)>,
    pending: Option<Pending>,
}

pub struct Controller {
    ui: Weak<AppWindow>,
    state: RefCell<State>,
}

/// Wires a controller to `ui`, loading data from `data_path`. `on_dark_theme` is told whenever
/// the app's effective theme turns dark (true) or light (false), and once at start-up, so the
/// platform can match its own chrome (such as Android's status bar icons).
pub fn attach(
    ui: &AppWindow,
    data_path: PathBuf,
    on_dark_theme: impl Fn(bool) + 'static,
) -> Rc<Controller> {
    let store = Store::open(data_path);
    let theme = match store.data.settings.theme_mode {
        ThemeMode::System => 0,
        ThemeMode::Light => 1,
        ThemeMode::Dark => 2,
    };
    ui.global::<Theme>().set_mode(theme);
    let c = Rc::new(Controller {
        ui: ui.as_weak(),
        state: RefCell::new(State {
            store,
            selected_month: start_of_month(now()),
            collapsed: HashSet::new(),
            sankey_width: 360.0,
            pies: Default::default(),
            top_category_ids: Vec::new(),
            editing_transaction: None,
            editing_budget: None,
            editing_category: None,
            category_parent_ids: Vec::new(),
            alloc_available: 0,
            alloc_amounts: Vec::new(),
            withdraw_options: Vec::new(),
            pending: None,
        }),
    });
    c.bind(ui);
    c.refresh();
    on_dark_theme(ui.global::<Theme>().get_dark());
    ui.global::<App>().on_dark_theme_changed(on_dark_theme);
    c
}

fn to_date(millis: Millis) -> Date {
    let (y, m, d) = ymd(millis);
    Date {
        year: y,
        month: m as i32 + 1,
        day: d as i32,
    }
}

fn from_date(d: &Date) -> Millis {
    local_midnight(d.year, d.month - 1, d.day.max(1) as u32)
}

fn options(first: &str, rest: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    model(
        std::iter::once(first.to_string())
            .chain(rest)
            .map(SharedString::from)
            .collect(),
    )
}

fn index_of(ids: &[Option<String>], id: Option<&str>) -> i32 {
    ids.iter().position(|x| x.as_deref() == id).unwrap_or(0) as i32
}

impl Controller {
    fn bind(self: &Rc<Self>, ui: &AppWindow) {
        let app = ui.global::<App>();
        macro_rules! on {
            ($setter:ident, |$c:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(self);
                app.$setter(move |$($arg),*| {
                    let $c = weak.upgrade().expect("controller outlives the window");
                    $body
                });
            }};
        }

        on!(on_prev_month, |c| c.shift_month(-1));
        on!(on_next_month, |c| c.shift_month(1));
        on!(on_cycle_theme, |c| c.cycle_theme());
        on!(on_page_changed, |c, _page| c.close_sheet());

        on!(on_add_transaction, |c| c.open_transaction(None));
        on!(on_edit_transaction, |c, id| c
            .open_transaction(Some(id.to_string())));
        on!(on_delete_transaction, |c, id| c.confirm(
            "Delete this transaction?",
            Pending::Transaction(id.to_string())
        ));
        on!(on_submit_transaction, |c| c.submit_transaction());

        on!(on_add_budget, |c| c.open_budget(None));
        on!(on_edit_budget, |c, id| c.open_budget(Some(id.to_string())));
        on!(on_delete_budget, |c, id| c.confirm(
            "Delete this budget?",
            Pending::Budget(id.to_string())
        ));
        on!(on_submit_budget, |c| c.submit_budget());

        on!(on_add_category, |c| c.open_category(None, None));
        on!(on_add_subcategory, |c, parent| c
            .open_category(None, Some(parent.to_string())));
        on!(on_edit_category, |c, id| c
            .open_category(Some(id.to_string()), None));
        on!(on_delete_category, |c, id| c
            .request_delete_category(id.to_string()));
        on!(on_move_category, |c, id, up| c.save(|s| s.move_category(
            &id,
            if up { Direction::Up } else { Direction::Down }
        )));
        on!(on_toggle_category, |c, id| {
            {
                let mut st = c.state.borrow_mut();
                let id = id.to_string();
                if !st.collapsed.remove(&id) {
                    st.collapsed.insert(id);
                }
            }
            c.refresh();
        });
        on!(on_submit_category, |c| c.submit_category());

        on!(on_open_allocate, |c| c.open_allocate());
        on!(on_alloc_edited, |c, index, text| c
            .alloc_edited(index as usize, text.to_string()));
        on!(on_submit_allocate, |c| c.submit_allocate());
        on!(on_open_withdraw, |c| c.open_withdraw());
        on!(on_submit_withdraw, |c| c.submit_withdraw());
        on!(on_wd_status, |c, index, amount| c
            .withdraw_status(index, &amount)
            .0
            .into());
        on!(on_wd_valid, |c, index, amount| c
            .withdraw_status(index, &amount)
            .1);

        on!(on_close_sheet, |c| c.close_sheet());
        on!(on_dialog_confirmed, |c| c.dialog_confirmed());
        on!(on_dialog_cancelled, |c| {
            c.state.borrow_mut().pending = None;
            c.with_app(|app| app.set_dialog_visible(false));
        });

        on!(on_pie_type_changed, |c, _kind| c.refresh());
        on!(on_sankey_resized, |c, width| {
            let width = width as f64;
            let changed = (c.state.borrow().sankey_width - width).abs() > 0.5;
            if changed && width > 0.0 {
                c.state.borrow_mut().sankey_width = width;
                c.refresh_sankey();
            }
        });
        on!(on_pie_hit, |c, id, x, y| {
            let st = c.state.borrow();
            st.pies
                .get(id as usize)
                .and_then(|p| p.hit_test(x as f64, y as f64))
                .map_or(-1, |i| i as i32)
        });
        on!(on_format_date, |_c, d| format_date(from_date(&d)).into());
        on!(on_monthly_hint, |_c, amount| {
            let monthly = monthly_equivalent_amount(
                parse_to_cents(&amount),
                Some(RecurrenceFrequency::Yearly),
            );
            format!("= {}/month", format_cents(monthly)).into()
        });
        on!(on_subcategory_options, |c, index| {
            let st = c.state.borrow();
            let parent = st.top_category_ids.get(index as usize).cloned().flatten();
            options(
                "None",
                children(&st.store.data.categories, parent.as_deref()).map(|c| c.name.clone()),
            )
        });
    }

    fn with_app(&self, f: impl FnOnce(App<'_>)) {
        if let Some(ui) = self.ui.upgrade() {
            f(ui.global::<App>());
        }
    }

    /// Runs a store mutation, reporting a failed save, then refreshes the UI.
    fn save(&self, f: impl FnOnce(&mut Store) -> std::io::Result<()>) {
        let result = f(&mut self.state.borrow_mut().store);
        if let Err(e) = result {
            self.alert(&format!("Couldn't save your changes: {e}"));
        }
        self.refresh();
    }

    fn shift_month(&self, delta: i32) {
        {
            let mut st = self.state.borrow_mut();
            let next = shift_month(st.selected_month, delta);
            if next > start_of_month(now()) {
                return;
            }
            st.selected_month = next;
        }
        self.refresh();
    }

    fn cycle_theme(&self) {
        let next = self.state.borrow().store.data.settings.theme_mode.next();
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Theme>().set_mode(match next {
                ThemeMode::System => 0,
                ThemeMode::Light => 1,
                ThemeMode::Dark => 2,
            });
        }
        self.save(|s| s.set_theme_mode(next));
    }

    /// Recomputes every view model from the data and pushes it to the window.
    fn refresh(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let app = ui.global::<App>();
        let mut st = self.state.borrow_mut();
        let month = st.selected_month;
        let data = &st.store.data;
        let in_month = month_view(&data.transactions, month);
        let summary = viewmodel::month_summary(&in_month);

        app.set_month_label(format_month_year(month).into());
        app.set_can_next_month(month < start_of_month(now()));
        app.set_today(to_date(now()));

        app.set_stats(model(viewmodel::stats(&summary)));
        app.set_can_allocate(summary.remaining > 0);
        app.set_can_withdraw(
            !withdrawable_budgets(&data.budgets, &data.transactions, month).is_empty(),
        );
        app.set_recurring_total(format_cents(summary.recurring_expense).into());
        app.set_onetime_total(format_cents(summary.one_time_expense).into());

        let cats = &data.categories;
        let expense = TransactionType::Expense;
        let pie_kind = if app.get_pie_type() == 1 {
            TransactionType::Income
        } else {
            expense
        };
        let pies = [
            viewmodel::pie(
                0,
                &viewmodel::breakdown(&viewmodel::expenses(&summary, None), cats, expense),
            ),
            viewmodel::pie(
                1,
                &viewmodel::breakdown(&viewmodel::expenses(&summary, Some(true)), cats, expense),
            ),
            viewmodel::pie(
                2,
                &viewmodel::breakdown(&viewmodel::expenses(&summary, Some(false)), cats, expense),
            ),
            viewmodel::pie(3, &viewmodel::breakdown(&in_month, cats, pie_kind)),
        ];
        let [(p0, l0), (p1, l1), (p2, l2), (p3, l3)] = pies;
        app.set_expense_pie(p0);
        app.set_recurring_pie(p1);
        app.set_onetime_pie(p2);
        app.set_charts_pie(p3);

        app.set_budget_cards(model(viewmodel::budget_cards(
            &data.budgets,
            cats,
            &data.transactions,
            month,
        )));
        app.set_tx_rows(model(viewmodel::transaction_rows(
            &data.transactions,
            cats,
            &data.budgets,
            month,
        )));
        app.set_category_rows(model(viewmodel::category_rows(cats, &st.collapsed)));

        let (nodes, links) = viewmodel::sankey(&in_month, cats, &data.budgets, st.sankey_width);
        app.set_sankey_nodes(model(nodes));
        app.set_sankey_links(model(links));

        // Form options track the data, so an open form sees renames immediately.
        let tops: Vec<&Category> = sort_categories(cats.iter().filter(|c| c.parent_id.is_none()));
        app.set_category_options(options(
            "Uncategorized",
            tops.iter().map(|c| c.name.clone()),
        ));
        app.set_budget_category_options(options("General", tops.iter().map(|c| c.name.clone())));
        let budget_names: Vec<String> = data.budgets.iter().map(|b| b.name.clone()).collect();
        app.set_budget_options(options("None", budget_names.clone()));
        app.set_budget_options_required(options("Select a budget", budget_names));
        st.top_category_ids = std::iter::once(None)
            .chain(tops.iter().map(|c| Some(c.id.clone())))
            .collect();
        st.pies = [l0, l1, l2, l3];
    }

    fn refresh_sankey(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let st = self.state.borrow();
        let data = &st.store.data;
        let in_month = month_view(&data.transactions, st.selected_month);
        let (nodes, links) =
            viewmodel::sankey(&in_month, &data.categories, &data.budgets, st.sankey_width);
        let app = ui.global::<App>();
        app.set_sankey_nodes(model(nodes));
        app.set_sankey_links(model(links));
    }

    fn open_sheet(&self, sheet: Sheet, title: &str) {
        self.with_app(|app| {
            app.set_sheet_title(title.into());
            app.set_sheet(sheet as i32);
        });
    }

    fn close_sheet(&self) {
        self.with_app(|app| app.set_sheet(Sheet::Closed as i32));
    }

    /// Date for fills and withdrawals made from the dashboard: today when viewing the current
    /// month, the viewed month's first day otherwise, so they land in the totals on screen.
    fn date_in_viewed_month(&self) -> Millis {
        let month = self.state.borrow().selected_month;
        let now = now();
        if start_of_month(now) == month {
            now
        } else {
            month
        }
    }

    // --- Transactions -----------------------------------------------------------------------

    fn open_transaction(&self, id: Option<String>) {
        self.refresh();
        let st = self.state.borrow();
        let t: Option<&Transaction> = id
            .as_deref()
            .and_then(|id| st.store.data.transactions.iter().find(|t| t.id == id));
        if id.is_some() && t.is_none() {
            return;
        }
        let month = st.selected_month;
        let budgets = &st.store.data.budgets;
        let category = t.and_then(|t| t.category_id.as_deref());
        let sub_ids: Vec<Option<String>> = std::iter::once(None)
            .chain(children(&st.store.data.categories, category).map(|c| Some(c.id.clone())))
            .collect();
        // Editing the amount of a recurring transaction that already occurred in earlier months
        // only applies from the viewed month on (the record is split to keep history).
        let split_hint = t
            .filter(|t| {
                t.recurrence.is_some()
                    && start_of_month(t.date) < month
                    && t.recurrence_end.is_none_or(|end| month < end)
            })
            .map(|t| {
                format!(
                    "A new amount applies from {} on; earlier months keep {}.",
                    format_month_year(month),
                    format_cents(t.amount)
                )
            })
            .unwrap_or_default();
        self.with_app(|app| {
            app.set_tx_editing(t.is_some());
            app.set_tx_type(match t.map(|t| t.kind) {
                Some(TransactionType::Income) => 1,
                Some(TransactionType::Allocation) => 2,
                Some(TransactionType::Withdrawal) => 3,
                _ => 0,
            });
            app.set_tx_amount(cents_to_input(t.map_or(0, |t| t.amount)).into());
            app.set_tx_recurrence(match t.and_then(|t| t.recurrence) {
                Some(RecurrenceFrequency::Monthly) => 1,
                Some(RecurrenceFrequency::Yearly) => 2,
                None => 0,
            });
            app.set_tx_date(to_date(t.map_or_else(now, |t| t.date)));
            app.set_tx_category(index_of(&st.top_category_ids, category));
            app.set_tx_subcategory(index_of(
                &sub_ids,
                t.and_then(|t| t.subcategory_id.as_deref()),
            ));
            let budget_ids: Vec<Option<String>> = std::iter::once(None)
                .chain(budgets.iter().map(|b| Some(b.id.clone())))
                .collect();
            app.set_tx_budget(index_of(
                &budget_ids,
                t.and_then(|t| t.budget_id.as_deref()),
            ));
            app.set_tx_note(t.map_or("", |t| t.note.as_str()).into());
            app.set_tx_split_hint(split_hint.into());
            app.set_tx_budget_missing(false);
        });
        drop(st);
        self.state.borrow_mut().editing_transaction = id;
        self.open_sheet(
            Sheet::Transaction,
            if self.state.borrow().editing_transaction.is_some() {
                "Edit transaction"
            } else {
                "Add transaction"
            },
        );
    }

    fn submit_transaction(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let app = ui.global::<App>();
        let kind = match app.get_tx_type() {
            1 => TransactionType::Income,
            2 => TransactionType::Allocation,
            3 => TransactionType::Withdrawal,
            _ => TransactionType::Expense,
        };
        let input = {
            let st = self.state.borrow();
            let budget_id = match app.get_tx_budget() {
                i if i > 0 => st
                    .store
                    .data
                    .budgets
                    .get(i as usize - 1)
                    .map(|b| b.id.clone()),
                _ => None,
            };
            // A fill has to land somewhere, and a withdrawal come from somewhere.
            if kind.is_budget_transfer() && budget_id.is_none() {
                app.set_tx_budget_missing(true);
                return;
            }
            let (category_id, subcategory_id) = if kind.is_budget_transfer() {
                (None, None)
            } else {
                let category = st
                    .top_category_ids
                    .get(app.get_tx_category() as usize)
                    .cloned()
                    .flatten();
                let sub = match app.get_tx_subcategory() {
                    i if i > 0 => children(&st.store.data.categories, category.as_deref())
                        .nth(i as usize - 1)
                        .map(|c| c.id.clone()),
                    _ => None,
                };
                (category, sub)
            };
            let recurrence = match app.get_tx_recurrence() {
                1 => Some(RecurrenceFrequency::Monthly),
                2 => Some(RecurrenceFrequency::Yearly),
                _ => None,
            };
            let existing_end = st
                .editing_transaction
                .as_deref()
                .and_then(|id| st.store.data.transactions.iter().find(|t| t.id == id))
                .and_then(|t| t.recurrence_end);
            NewTransaction {
                kind,
                amount: parse_to_cents(&app.get_tx_amount()),
                date: from_date(&app.get_tx_date()),
                category_id,
                subcategory_id,
                budget_id: if kind == TransactionType::Income {
                    None
                } else {
                    budget_id
                },
                note: app.get_tx_note().trim().to_string(),
                recurrence,
                recurrence_end: recurrence.and(existing_end),
            }
        };
        let (editing, month) = {
            let st = self.state.borrow();
            (st.editing_transaction.clone(), st.selected_month)
        };
        self.close_sheet();
        match editing {
            Some(id) => self.save(|s| s.update_transaction(&id, input, month)),
            None => self.save(|s| s.add_transaction(input)),
        }
    }

    // --- Budgets ----------------------------------------------------------------------------

    fn open_budget(&self, id: Option<String>) {
        self.refresh();
        let st = self.state.borrow();
        let b: Option<&Budget> = id
            .as_deref()
            .and_then(|id| st.store.data.budgets.iter().find(|b| b.id == id));
        if id.is_some() && b.is_none() {
            return;
        }
        let category = b.and_then(|b| b.category_id.as_deref());
        let sub_ids: Vec<Option<String>> = std::iter::once(None)
            .chain(children(&st.store.data.categories, category).map(|c| Some(c.id.clone())))
            .collect();
        self.with_app(|app| {
            app.set_bf_editing(b.is_some());
            app.set_bf_name(b.map_or("", |b| b.name.as_str()).into());
            app.set_bf_description(b.map_or("", |b| b.description.as_str()).into());
            app.set_bf_amount(cents_to_input(b.map_or(0, |b| b.target_amount)).into());
            app.set_bf_period(i32::from(
                b.is_some_and(|b| b.period_type == BudgetPeriodType::OneTime),
            ));
            app.set_bf_start(to_date(b.map_or_else(now, |b| b.start_date)));
            let end = b.and_then(|b| b.end_date);
            app.set_bf_has_end(end.is_some());
            app.set_bf_end(to_date(end.unwrap_or_else(now)));
            app.set_bf_category(index_of(&st.top_category_ids, category));
            app.set_bf_subcategory(index_of(
                &sub_ids,
                b.and_then(|b| b.subcategory_id.as_deref()),
            ));
            app.set_bf_name_missing(false);
        });
        drop(st);
        let title = if id.is_some() {
            "Edit budget"
        } else {
            "Add budget"
        };
        self.state.borrow_mut().editing_budget = id;
        self.open_sheet(Sheet::Budget, title);
    }

    fn submit_budget(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let app = ui.global::<App>();
        let name = app.get_bf_name().trim().to_string();
        if name.is_empty() {
            app.set_bf_name_missing(true);
            return;
        }
        let input = {
            let st = self.state.borrow();
            let category = st
                .top_category_ids
                .get(app.get_bf_category() as usize)
                .cloned()
                .flatten();
            let sub = if app.get_bf_subcategory() > 0 {
                children(&st.store.data.categories, category.as_deref())
                    .nth(app.get_bf_subcategory() as usize - 1)
                    .map(|c| c.id.clone())
            } else {
                None
            };
            NewBudget {
                name,
                description: app.get_bf_description().trim().to_string(),
                target_amount: parse_to_cents(&app.get_bf_amount()),
                period_type: if app.get_bf_period() == 1 {
                    BudgetPeriodType::OneTime
                } else {
                    BudgetPeriodType::Monthly
                },
                start_date: from_date(&app.get_bf_start()),
                end_date: app.get_bf_has_end().then(|| from_date(&app.get_bf_end())),
                category_id: category,
                subcategory_id: sub,
            }
        };
        let editing = self.state.borrow().editing_budget.clone();
        self.close_sheet();
        match editing {
            Some(id) => self.save(|s| s.update_budget(&id, input)),
            None => self.save(|s| s.add_budget(input)),
        }
    }

    // --- Categories -------------------------------------------------------------------------

    fn open_category(&self, id: Option<String>, preset_parent: Option<String>) {
        let st = self.state.borrow();
        let c: Option<&Category> = id
            .as_deref()
            .and_then(|id| st.store.data.categories.iter().find(|c| c.id == id));
        if id.is_some() && c.is_none() {
            return;
        }
        let parents: Vec<&Category> = sort_categories(
            st.store
                .data
                .categories
                .iter()
                .filter(|p| p.parent_id.is_none() && Some(&p.id) != c.map(|c| &c.id)),
        );
        let parent_ids: Vec<Option<String>> = std::iter::once(None)
            .chain(parents.iter().map(|p| Some(p.id.clone())))
            .collect();
        let selected_parent = c.map_or(preset_parent.as_deref(), |c| c.parent_id.as_deref());
        self.with_app(|app| {
            app.set_cf_editing(c.is_some());
            app.set_cf_name(c.map_or("", |c| c.name.as_str()).into());
            app.set_cf_parent_options(options(
                "None (top-level)",
                parents.iter().map(|p| p.name.clone()),
            ));
            app.set_cf_parent(index_of(&parent_ids, selected_parent));
            app.set_cf_color(parse_color(
                c.map_or(DEFAULT_CATEGORY_COLOR, |c| c.color.as_str()),
            ));
            app.set_cf_name_missing(false);
        });
        drop(st);
        let title = if id.is_some() {
            "Edit category"
        } else if preset_parent.is_some() {
            "Add subcategory"
        } else {
            "Add category"
        };
        {
            let mut st = self.state.borrow_mut();
            st.editing_category = id;
            st.category_parent_ids = parent_ids;
        }
        self.open_sheet(Sheet::Category, title);
    }

    fn submit_category(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let app = ui.global::<App>();
        let name = app.get_cf_name().trim().to_string();
        if name.is_empty() {
            app.set_cf_name_missing(true);
            return;
        }
        let (editing, parent_id) = {
            let st = self.state.borrow();
            (
                st.editing_category.clone(),
                st.category_parent_ids
                    .get(app.get_cf_parent() as usize)
                    .cloned()
                    .flatten(),
            )
        };
        let input = NewCategory {
            name,
            parent_id,
            color: viewmodel::color_to_hex(app.get_cf_color()),
        };
        self.close_sheet();
        match editing {
            Some(id) => self.save(|s| s.update_category(&id, input)),
            None => self.save(|s| s.add_category(input)),
        }
    }

    fn request_delete_category(&self, id: String) {
        if self.state.borrow().store.has_subcategories(&id) {
            self.alert("Move or delete subcategories first.");
        } else {
            self.confirm("Delete this category?", Pending::Category(id));
        }
    }

    // --- Allocate / withdraw ----------------------------------------------------------------

    fn open_allocate(&self) {
        self.refresh();
        let (available, names) = {
            let st = self.state.borrow();
            let in_month = month_view(&st.store.data.transactions, st.selected_month);
            let remaining = viewmodel::month_summary(&in_month).remaining;
            (
                remaining,
                st.store
                    .data
                    .budgets
                    .iter()
                    .map(|b| b.name.clone())
                    .collect::<Vec<_>>(),
            )
        };
        {
            let mut st = self.state.borrow_mut();
            st.alloc_available = available;
            st.alloc_amounts = vec![String::new(); names.len()];
        }
        self.with_app(|app| {
            app.set_alloc_rows(model(
                names
                    .into_iter()
                    .map(|n| AllocRow {
                        name: n.into(),
                        amount: SharedString::new(),
                    })
                    .collect(),
            ));
        });
        self.update_alloc_remaining();
        self.open_sheet(Sheet::Allocate, "Allocate remaining funds");
    }

    fn alloc_edited(&self, index: usize, text: String) {
        if let Some(slot) = self.state.borrow_mut().alloc_amounts.get_mut(index) {
            *slot = text;
        }
        self.update_alloc_remaining();
    }

    /// Recomputes the "left to allocate" line and gates the submit button on it.
    fn update_alloc_remaining(&self) {
        let st = self.state.borrow();
        let total: Cents = st.alloc_amounts.iter().map(|a| parse_to_cents(a)).sum();
        let left = st.alloc_available - total;
        self.with_app(|app| {
            app.set_alloc_remaining(
                if left < 0 {
                    format!("Over by {}", format_cents(-left))
                } else {
                    format!("Left to allocate: {}", format_cents(left))
                }
                .into(),
            );
            app.set_alloc_over(left < 0);
            app.set_alloc_can_submit(total > 0 && left >= 0);
        });
    }

    fn submit_allocate(&self) {
        let date = self.date_in_viewed_month();
        let inputs: Vec<NewTransaction> = {
            let st = self.state.borrow();
            let amounts: Vec<Cents> = st.alloc_amounts.iter().map(|a| parse_to_cents(a)).collect();
            let total: Cents = amounts.iter().sum();
            if total <= 0 || total > st.alloc_available {
                return;
            }
            st.store
                .data
                .budgets
                .iter()
                .zip(amounts)
                .filter(|(_, cents)| *cents > 0)
                .map(|(b, cents)| transfer(TransactionType::Allocation, cents, date, &b.id))
                .collect()
        };
        self.close_sheet();
        self.save(|s| s.add_transactions(inputs));
    }

    fn open_withdraw(&self) {
        self.refresh();
        let options: Vec<(String, String, Cents)> = {
            let st = self.state.borrow();
            withdrawable_budgets(
                &st.store.data.budgets,
                &st.store.data.transactions,
                st.selected_month,
            )
            .into_iter()
            .map(|(b, balance)| (b.id.clone(), b.name.clone(), balance))
            .collect()
        };
        self.with_app(|app| {
            app.set_wd_options(model(
                options
                    .iter()
                    .map(|(_, name, bal)| format!("{name} ({})", format_cents(*bal)).into())
                    .collect(),
            ));
            app.set_wd_index(0);
            app.set_wd_amount(SharedString::new());
        });
        self.state.borrow_mut().withdraw_options =
            options.into_iter().map(|(id, _, bal)| (id, bal)).collect();
        self.open_sheet(Sheet::Withdraw, "Withdraw from budget");
    }

    /// The "left in budget" line for the chosen budget and amount, and whether it's submittable.
    fn withdraw_status(&self, index: i32, amount: &str) -> (String, bool) {
        let st = self.state.borrow();
        let available = st
            .withdraw_options
            .get(index.max(0) as usize)
            .map_or(0, |o| o.1);
        let cents = parse_to_cents(amount);
        let left = available - cents;
        let text = if left < 0 {
            format!("Over by {}", format_cents(-left))
        } else {
            format!("Left in budget: {}", format_cents(left))
        };
        (text, cents > 0 && left >= 0)
    }

    fn submit_withdraw(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let app = ui.global::<App>();
        let (index, amount) = (app.get_wd_index(), app.get_wd_amount());
        if !self.withdraw_status(index, &amount).1 {
            return;
        }
        let Some(budget_id) = self
            .state
            .borrow()
            .withdraw_options
            .get(index as usize)
            .map(|o| o.0.clone())
        else {
            return;
        };
        let input = transfer(
            TransactionType::Withdrawal,
            parse_to_cents(&amount),
            self.date_in_viewed_month(),
            &budget_id,
        );
        self.close_sheet();
        self.save(|s| s.add_transaction(input));
    }

    // --- Dialogs ----------------------------------------------------------------------------

    fn confirm(&self, message: &str, action: Pending) {
        self.state.borrow_mut().pending = Some(action);
        self.with_app(|app| {
            app.set_dialog_message(message.into());
            app.set_dialog_confirm_label("Delete".into());
            app.set_dialog_can_cancel(true);
            app.set_dialog_visible(true);
        });
    }

    fn alert(&self, message: &str) {
        self.state.borrow_mut().pending = None;
        self.with_app(|app| {
            app.set_dialog_message(message.into());
            app.set_dialog_confirm_label("OK".into());
            app.set_dialog_can_cancel(false);
            app.set_dialog_visible(true);
        });
    }

    fn dialog_confirmed(&self) {
        self.with_app(|app| app.set_dialog_visible(false));
        let pending = self.state.borrow_mut().pending.take();
        match pending {
            Some(Pending::Transaction(id)) => self.save(|s| s.delete_transaction(&id)),
            Some(Pending::Category(id)) => self.save(|s| s.delete_category(&id)),
            Some(Pending::Budget(id)) => self.save(|s| s.delete_budget(&id)),
            None => {}
        }
    }
}

/// Subcategories of `parent`, in display order.
fn children<'a>(
    categories: &'a [Category],
    parent: Option<&str>,
) -> impl Iterator<Item = &'a Category> {
    let list = match parent {
        Some(p) => sort_categories(
            categories
                .iter()
                .filter(|c| c.parent_id.as_deref() == Some(p)),
        ),
        None => Vec::new(),
    };
    list.into_iter()
}

/// A one-off fill or withdrawal: budget-linked, no category.
fn transfer(kind: TransactionType, amount: Cents, date: Millis, budget_id: &str) -> NewTransaction {
    NewTransaction {
        kind,
        amount,
        date,
        category_id: None,
        subcategory_id: None,
        budget_id: Some(budget_id.to_string()),
        note: String::new(),
        recurrence: None,
        recurrence_end: None,
    }
}
