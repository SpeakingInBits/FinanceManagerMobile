//! Data model. Field names serialize in camelCase so records match the web app's IndexedDB shape.

use serde::{Deserialize, Serialize};

/// Epoch milliseconds, matching JavaScript `Date.getTime()`.
pub type Millis = i64;
/// Integer minor units (cents) to avoid floating point drift.
pub type Cents = i64;

/// - `Income`: money earned (a paycheck); never linked to a budget.
/// - `Expense`: money spent; deducts from a budget when linked to one.
/// - `Allocation`: a budget fill: moves money from already-logged income into a budget. Always
///   linked to a budget and carries no category.
/// - `Withdrawal`: the reverse of a fill: moves money out of a budget's balance back into general
///   income, where it can be spent or allocated elsewhere. Always linked to a budget and carries
///   no category.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TransactionType {
    Income,
    Expense,
    Allocation,
    Withdrawal,
}

impl TransactionType {
    /// Fills and withdrawals move money into or out of a budget and carry no category.
    pub fn is_budget_transfer(self) -> bool {
        matches!(self, Self::Allocation | Self::Withdrawal)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum RecurrenceFrequency {
    Monthly,
    Yearly,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: TransactionType,
    /// For recurring transactions, this is the per-occurrence amount (e.g. the full yearly bill),
    /// not the monthly-equivalent.
    pub amount: Cents,
    /// For recurring transactions, the anchor/start date; it recurs on this day-of-month from
    /// here on.
    pub date: Millis,
    pub category_id: Option<String>,
    #[serde(default)]
    pub subcategory_id: Option<String>,
    pub budget_id: Option<String>,
    #[serde(default)]
    pub note: String,
    /// `None` = one-off transaction.
    #[serde(default)]
    pub recurrence: Option<RecurrenceFrequency>,
    /// Start-of-month millis of the first month this recurrence no longer occurs in (exclusive
    /// end); `None` = recurs indefinitely. Always `None` for one-offs. Set when an edit to a
    /// recurring transaction is carried forward as a new record so earlier months keep their
    /// history.
    #[serde(default)]
    pub recurrence_end: Option<Millis>,
    pub created_at: Millis,
    pub updated_at: Millis,
}

/// A transaction as entered in a form, before it gets an id and timestamps.
#[derive(Clone, PartialEq, Debug)]
pub struct NewTransaction {
    pub kind: TransactionType,
    pub amount: Cents,
    pub date: Millis,
    pub category_id: Option<String>,
    pub subcategory_id: Option<String>,
    pub budget_id: Option<String>,
    pub note: String,
    pub recurrence: Option<RecurrenceFrequency>,
    pub recurrence_end: Option<Millis>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    /// `None` = top-level category.
    pub parent_id: Option<String>,
    /// Hex color, reused consistently across list swatches and charts.
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Sibling display order; falls back to `created_at` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i64>,
    pub created_at: Millis,
}

#[derive(Clone, PartialEq, Debug)]
pub struct NewCategory {
    pub name: String,
    pub parent_id: Option<String>,
    pub color: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetPeriodType {
    Monthly,
    OneTime,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub id: String,
    pub name: String,
    /// Free-form notes about this budget. Empty when unset.
    #[serde(default)]
    pub description: String,
    /// Contribution goal for this budget's period window (not a spending cap).
    pub target_amount: Cents,
    pub period_type: BudgetPeriodType,
    /// For monthly budgets, the month it starts recurring.
    pub start_date: Millis,
    /// End of a one-time budget, or an explicit end to a monthly budget.
    pub end_date: Option<Millis>,
    /// `None` = general budget not tied to one category.
    pub category_id: Option<String>,
    /// `None` = not narrowed to a subcategory; must be a child of `category_id` when set.
    #[serde(default)]
    pub subcategory_id: Option<String>,
    pub created_at: Millis,
}

#[derive(Clone, PartialEq, Debug)]
pub struct NewBudget {
    pub name: String,
    pub description: String,
    pub target_amount: Cents,
    pub period_type: BudgetPeriodType,
    pub start_date: Millis,
    pub end_date: Option<Millis>,
    pub category_id: Option<String>,
    pub subcategory_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    /// The next mode in the header toggle's cycle: system → light → dark → system.
    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub theme_mode: ThemeMode,
}
