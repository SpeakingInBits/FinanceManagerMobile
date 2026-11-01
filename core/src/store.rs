//! Local-only persistence (a single JSON document, written atomically) and the mutating actions
//! the UI performs, mirroring the web app's repos + `actions.ts`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::date::{now, start_of_month};
use crate::model::{
    Budget, Category, Millis, NewBudget, NewCategory, NewTransaction, Settings, ThemeMode,
    Transaction,
};
use crate::recurrence::project_day_into_month;

/// Bumped whenever the stored shape changes; [`Data::migrate`] upgrades older documents.
pub const DATA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Data {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub transactions: Vec<Transaction>,
    #[serde(default)]
    pub categories: Vec<Category>,
    #[serde(default)]
    pub budgets: Vec<Budget>,
    #[serde(default)]
    pub settings: Settings,
}

impl Data {
    fn migrate(mut self) -> Self {
        // Future migrations: if self.version < 2 { ... }; never edit earlier steps.
        self.version = DATA_VERSION;
        self
    }
}

/// Stable sort by explicit `order` when set, falling back to creation order.
pub fn sort_categories<'a>(
    categories: impl IntoIterator<Item = &'a Category>,
) -> Vec<&'a Category> {
    let mut v: Vec<&Category> = categories.into_iter().collect();
    v.sort_by_key(|c| c.order.unwrap_or(c.created_at));
    v
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Up,
    Down,
}

/// In-memory app data plus the file it is saved to (none in tests).
pub struct Store {
    path: Option<PathBuf>,
    pub data: Data,
}

impl Store {
    /// Opens the data file at `path`, starting empty if it doesn't exist yet. A corrupt file is
    /// moved aside (never silently overwritten) and the app starts fresh.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let data = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Data>(&bytes) {
                Ok(data) => data.migrate(),
                Err(_) => {
                    let _ = fs::rename(&path, path.with_extension("corrupt.json"));
                    Data::default().migrate()
                }
            },
            Err(_) => Data::default().migrate(),
        };
        Self {
            path: Some(path),
            data,
        }
    }

    pub fn in_memory(data: Data) -> Self {
        Self {
            path: None,
            data: data.migrate(),
        }
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        write_atomic(path, &serde_json::to_vec(&self.data)?)
    }

    // --- Transactions ---------------------------------------------------------------------

    pub fn add_transaction(&mut self, input: NewTransaction) -> io::Result<()> {
        let t = new_transaction(input, now());
        self.data.transactions.push(t);
        self.save()
    }

    /// Applies an edit. Amount and frequency changes to a recurring transaction describe a new
    /// going-forward reality, so they take effect from the later of `selected_month` and the
    /// submitted date's month: the old record is closed just before that month and the edit is
    /// carried forward as a new record, keeping earlier months' history intact.
    pub fn update_transaction(
        &mut self,
        id: &str,
        input: NewTransaction,
        selected_month: Millis,
    ) -> io::Result<()> {
        let Some(index) = self.data.transactions.iter().position(|t| t.id == id) else {
            return Ok(());
        };
        let time = now();
        let existing = &mut self.data.transactions[index];
        match recurring_split_month(existing, &input, selected_month) {
            Some(split) => {
                let carried_end = existing.recurrence_end;
                existing.recurrence_end = Some(split);
                existing.updated_at = time;
                let forward = NewTransaction {
                    date: project_day_into_month(input.date, split),
                    recurrence_end: if input.recurrence.is_none() {
                        None
                    } else {
                        carried_end
                    },
                    ..input
                };
                self.data.transactions.push(new_transaction(forward, time));
            }
            None => apply_transaction(existing, input, time),
        }
        self.save()
    }

    pub fn delete_transaction(&mut self, id: &str) -> io::Result<()> {
        self.data.transactions.retain(|t| t.id != id);
        self.save()
    }

    /// Adds several transactions with one write (the dashboard's "Allocate remaining funds").
    pub fn add_transactions(&mut self, inputs: Vec<NewTransaction>) -> io::Result<()> {
        let time = now();
        self.data
            .transactions
            .extend(inputs.into_iter().map(|i| new_transaction(i, time)));
        self.save()
    }

    // --- Categories -----------------------------------------------------------------------

    pub fn add_category(&mut self, input: NewCategory) -> io::Result<()> {
        self.data.categories.push(Category {
            id: uuid::Uuid::new_v4().to_string(),
            name: input.name,
            parent_id: input.parent_id,
            color: input.color,
            icon: None,
            order: None,
            created_at: now(),
        });
        self.save()
    }

    pub fn update_category(&mut self, id: &str, input: NewCategory) -> io::Result<()> {
        if let Some(c) = self.data.categories.iter_mut().find(|c| c.id == id) {
            c.name = input.name;
            c.parent_id = input.parent_id;
            c.color = input.color;
        }
        self.save()
    }

    pub fn delete_category(&mut self, id: &str) -> io::Result<()> {
        self.data.categories.retain(|c| c.id != id);
        self.save()
    }

    pub fn has_subcategories(&self, id: &str) -> bool {
        self.data
            .categories
            .iter()
            .any(|c| c.parent_id.as_deref() == Some(id))
    }

    /// Swaps a category with its neighbor among its siblings, first normalizing sibling `order`
    /// values to 0..n when any is unset.
    pub fn move_category(&mut self, id: &str, direction: Direction) -> io::Result<()> {
        let Some(target) = self.data.categories.iter().find(|c| c.id == id) else {
            return Ok(());
        };
        let parent = target.parent_id.clone();
        let siblings: Vec<(String, Option<i64>)> = sort_categories(
            self.data
                .categories
                .iter()
                .filter(|c| c.parent_id == parent),
        )
        .into_iter()
        .map(|c| (c.id.clone(), c.order))
        .collect();
        let Some(index) = siblings.iter().position(|(sid, _)| sid == id) else {
            return Ok(());
        };
        let neighbor = match direction {
            Direction::Up if index > 0 => index - 1,
            Direction::Down if index + 1 < siblings.len() => index + 1,
            _ => return Ok(()),
        };
        let normalize = siblings.iter().any(|(_, o)| o.is_none());
        let orders: Vec<i64> = siblings
            .iter()
            .enumerate()
            .map(|(i, (_, o))| if normalize { i as i64 } else { o.unwrap_or(0) })
            .collect();
        let mut updates: Vec<(&str, i64)> = Vec::new();
        if normalize {
            updates.extend(
                siblings
                    .iter()
                    .zip(&orders)
                    .map(|((sid, _), o)| (sid.as_str(), *o)),
            );
        }
        updates.push((siblings[index].0.as_str(), orders[neighbor]));
        updates.push((siblings[neighbor].0.as_str(), orders[index]));
        for (sid, order) in updates {
            if let Some(c) = self.data.categories.iter_mut().find(|c| c.id == sid) {
                c.order = Some(order);
            }
        }
        self.save()
    }

    // --- Budgets --------------------------------------------------------------------------

    pub fn add_budget(&mut self, input: NewBudget) -> io::Result<()> {
        self.data.budgets.push(Budget {
            id: uuid::Uuid::new_v4().to_string(),
            name: input.name,
            description: input.description,
            target_amount: input.target_amount,
            period_type: input.period_type,
            start_date: input.start_date,
            end_date: input.end_date,
            category_id: input.category_id,
            subcategory_id: input.subcategory_id,
            created_at: now(),
        });
        self.save()
    }

    pub fn update_budget(&mut self, id: &str, input: NewBudget) -> io::Result<()> {
        if let Some(b) = self.data.budgets.iter_mut().find(|b| b.id == id) {
            b.name = input.name;
            b.description = input.description;
            b.target_amount = input.target_amount;
            b.period_type = input.period_type;
            b.start_date = input.start_date;
            b.end_date = input.end_date;
            b.category_id = input.category_id;
            b.subcategory_id = input.subcategory_id;
        }
        self.save()
    }

    pub fn delete_budget(&mut self, id: &str) -> io::Result<()> {
        self.data.budgets.retain(|b| b.id != id);
        self.save()
    }

    // --- Settings -------------------------------------------------------------------------

    pub fn set_theme_mode(&mut self, mode: ThemeMode) -> io::Result<()> {
        self.data.settings.theme_mode = mode;
        self.save()
    }
}

fn new_transaction(input: NewTransaction, time: Millis) -> Transaction {
    let mut t = Transaction {
        id: uuid::Uuid::new_v4().to_string(),
        kind: input.kind,
        amount: 0,
        date: 0,
        category_id: None,
        subcategory_id: None,
        budget_id: None,
        note: String::new(),
        recurrence: None,
        recurrence_end: None,
        created_at: time,
        updated_at: time,
    };
    apply_transaction(&mut t, input, time);
    t
}

fn apply_transaction(t: &mut Transaction, input: NewTransaction, time: Millis) {
    t.kind = input.kind;
    t.amount = input.amount;
    t.date = input.date;
    t.category_id = input.category_id;
    t.subcategory_id = input.subcategory_id;
    t.budget_id = input.budget_id;
    t.note = input.note;
    t.recurrence = input.recurrence;
    t.recurrence_end = input.recurrence_end;
    t.updated_at = time;
}

/// The month a history-rewriting edit to a recurring transaction takes effect from, or `None`
/// when the edit can be applied in place.
fn recurring_split_month(
    existing: &Transaction,
    input: &NewTransaction,
    selected_month: Millis,
) -> Option<Millis> {
    existing.recurrence?;
    if input.amount == existing.amount && input.recurrence == existing.recurrence {
        return None;
    }
    let effective = start_of_month(selected_month).max(start_of_month(input.date));
    if effective <= start_of_month(existing.date) {
        return None;
    }
    if existing.recurrence_end.is_some_and(|end| effective >= end) {
        return None;
    }
    Some(effective)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::local_midnight;
    use crate::model::{RecurrenceFrequency, TransactionType};

    fn input(amount: i64, date: Millis, recurrence: Option<RecurrenceFrequency>) -> NewTransaction {
        NewTransaction {
            kind: TransactionType::Expense,
            amount,
            date,
            category_id: None,
            subcategory_id: None,
            budget_id: None,
            note: "Rent".into(),
            recurrence,
            recurrence_end: None,
        }
    }

    #[test]
    fn editing_recurring_amount_preserves_history() {
        let mut store = Store::in_memory(Data::default());
        let anchor = local_midnight(2026, 0, 15);
        store
            .add_transaction(input(1000, anchor, Some(RecurrenceFrequency::Monthly)))
            .unwrap();
        let id = store.data.transactions[0].id.clone();

        let viewing = local_midnight(2026, 3, 1);
        store
            .update_transaction(
                &id,
                input(1200, anchor, Some(RecurrenceFrequency::Monthly)),
                viewing,
            )
            .unwrap();

        assert_eq!(store.data.transactions.len(), 2);
        let old = &store.data.transactions[0];
        assert_eq!(old.amount, 1000);
        assert_eq!(old.recurrence_end, Some(viewing));
        let new = &store.data.transactions[1];
        assert_eq!(new.amount, 1200);
        assert_eq!(new.date, local_midnight(2026, 3, 15));
        assert_eq!(new.recurrence_end, None);
    }

    #[test]
    fn editing_note_or_anchor_month_updates_in_place() {
        let mut store = Store::in_memory(Data::default());
        let anchor = local_midnight(2026, 0, 15);
        store
            .add_transaction(input(1000, anchor, Some(RecurrenceFrequency::Monthly)))
            .unwrap();
        let id = store.data.transactions[0].id.clone();
        let mut edit = input(1000, anchor, Some(RecurrenceFrequency::Monthly));
        edit.note = "Rent (new landlord)".into();
        store
            .update_transaction(&id, edit, local_midnight(2026, 5, 1))
            .unwrap();
        // Viewing the anchor month itself: nothing earlier to preserve.
        store
            .update_transaction(
                &id,
                input(5, anchor, Some(RecurrenceFrequency::Monthly)),
                anchor,
            )
            .unwrap();
        assert_eq!(store.data.transactions.len(), 1);
        assert_eq!(store.data.transactions[0].amount, 5);
        assert_eq!(store.data.transactions[0].note, "Rent");
    }

    #[test]
    fn move_category_normalizes_then_swaps() {
        let mut store = Store::in_memory(Data::default());
        for name in ["A", "B", "C"] {
            store
                .add_category(NewCategory {
                    name: name.into(),
                    parent_id: None,
                    color: "#000".into(),
                })
                .unwrap();
            // Distinct creation times aren't guaranteed within one millisecond; force them.
            let n = store.data.categories.len() as i64;
            store.data.categories.last_mut().unwrap().created_at = n;
        }
        let c = store.data.categories[2].id.clone();
        store.move_category(&c, Direction::Up).unwrap();
        let names: Vec<&str> = sort_categories(&store.data.categories)
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["A", "C", "B"]);
        store.move_category(&c, Direction::Up).unwrap();
        store.move_category(&c, Direction::Up).unwrap(); // already first: no-op
        let names: Vec<&str> = sort_categories(&store.data.categories)
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["C", "A", "B"]);
    }

    #[test]
    fn persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        let mut store = Store::open(&path);
        store
            .add_category(NewCategory {
                name: "Food".into(),
                parent_id: None,
                color: "#ff0000".into(),
            })
            .unwrap();
        store.set_theme_mode(ThemeMode::Dark).unwrap();
        let reopened = Store::open(&path);
        assert_eq!(reopened.data.categories.len(), 1);
        assert_eq!(reopened.data.settings.theme_mode, ThemeMode::Dark);
        let json = fs::read_to_string(&path).unwrap();
        assert!(
            json.contains("\"parentId\":null"),
            "camelCase like the web app: {json}"
        );
    }

    #[test]
    fn corrupt_file_is_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        fs::write(&path, b"{not json").unwrap();
        let store = Store::open(&path);
        assert!(store.data.transactions.is_empty());
        assert!(dir.path().join("data.corrupt.json").exists());
    }
}
