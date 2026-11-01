//! Budget balance and progress calculations.

use crate::date::month_bounds;
use crate::model::{Budget, BudgetPeriodType, Cents, Millis, Transaction, TransactionType};
use crate::recurrence::occurrences_through_month;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BudgetStats {
    pub period_type: BudgetPeriodType,
    /// Lifetime balance: fills (allocations) into this budget minus expense and withdrawals,
    /// with recurring transactions counted once per elapsed month through the reference month.
    /// Can be negative.
    pub balance: Cents,
    pub overdrawn: bool,
    /// Money allocated into this budget within the contribution window of the reference month.
    pub contributed: Cents,
    pub target: Cents,
    /// Amount filling the progress bar toward `target`. Monthly budgets track this window's
    /// `contributed`; one-time budgets track the lifetime `balance`, so the bar shows how much of
    /// the goal is currently funded.
    pub progress: Cents,
    /// progress / target, clamped to [0, 999]; 0 when target <= 0.
    pub progress_percent: f64,
    pub progress_complete: bool,
}

/// Computes a budget's spendable balance and its progress toward `target_amount`, as of
/// `reference_month` (start-of-month millis). Monthly budgets progress by the amount allocated
/// within that month; one-time budgets progress by their lifetime balance.
///
/// Fills are `Allocation` transactions; budget-linked `Income` records (the pre-v7 way of
/// funding a budget) count the same way for resilience against unmigrated data. Withdrawals,
/// like expense, lower the balance but leave `contributed` untouched.
pub fn compute_budget_stats(
    budget: &Budget,
    transactions: &[Transaction],
    reference_month: Millis,
) -> BudgetStats {
    let (window_start, window_end) = match budget.period_type {
        BudgetPeriodType::Monthly => month_bounds(reference_month),
        BudgetPeriodType::OneTime => (budget.start_date, budget.end_date.unwrap_or(Millis::MAX)),
    };

    let mut funded = 0;
    let mut drawn = 0;
    let mut contributed = 0;

    for t in transactions {
        if t.budget_id.as_deref() != Some(budget.id.as_str()) {
            continue;
        }
        for o in occurrences_through_month(t, reference_month) {
            if matches!(
                t.kind,
                TransactionType::Expense | TransactionType::Withdrawal
            ) {
                drawn += o.display_amount;
            } else {
                funded += o.display_amount;
                if o.display_date >= window_start && o.display_date <= window_end {
                    contributed += o.display_amount;
                }
            }
        }
    }

    let balance = funded - drawn;
    let target = budget.target_amount;
    let progress = match budget.period_type {
        BudgetPeriodType::OneTime => balance,
        BudgetPeriodType::Monthly => contributed,
    };
    let progress_percent = if target > 0 {
        (progress as f64 / target as f64).clamp(0.0, 999.0)
    } else {
        0.0
    };

    BudgetStats {
        period_type: budget.period_type,
        balance,
        overdrawn: balance < 0,
        contributed,
        target,
        progress,
        progress_percent,
        progress_complete: target > 0 && progress >= target,
    }
}

/// Budgets with money to take out as of `reference_month`, paired with that balance.
pub fn withdrawable_budgets<'a>(
    budgets: &'a [Budget],
    transactions: &[Transaction],
    reference_month: Millis,
) -> Vec<(&'a Budget, Cents)> {
    budgets
        .iter()
        .map(|b| {
            (
                b,
                compute_budget_stats(b, transactions, reference_month).balance,
            )
        })
        .filter(|(_, balance)| *balance > 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::local_midnight;
    use crate::model::RecurrenceFrequency;
    use crate::recurrence::tests::tx;

    fn budget(period_type: BudgetPeriodType, target: Cents) -> Budget {
        Budget {
            id: "b1".into(),
            name: "Groceries".into(),
            description: String::new(),
            target_amount: target,
            period_type,
            start_date: local_midnight(2026, 0, 1),
            end_date: None,
            category_id: None,
            subcategory_id: None,
            created_at: 0,
        }
    }

    fn linked(kind: TransactionType, amount: Cents, date: Millis) -> Transaction {
        let mut t = tx(kind, amount, date);
        t.budget_id = Some("b1".into());
        t
    }

    #[test]
    fn monthly_budget_progress_is_this_months_fills() {
        let b = budget(BudgetPeriodType::Monthly, 10_000);
        let txs = vec![
            linked(
                TransactionType::Allocation,
                4_000,
                local_midnight(2026, 0, 5),
            ),
            linked(
                TransactionType::Allocation,
                6_000,
                local_midnight(2026, 1, 5),
            ),
            linked(TransactionType::Expense, 3_000, local_midnight(2026, 1, 9)),
            linked(
                TransactionType::Withdrawal,
                1_000,
                local_midnight(2026, 1, 10),
            ),
        ];
        let s = compute_budget_stats(&b, &txs, local_midnight(2026, 1, 1));
        assert_eq!(s.contributed, 6_000);
        assert_eq!(s.progress, 6_000);
        assert_eq!(s.balance, 6_000);
        assert!((s.progress_percent - 0.6).abs() < 1e-9);
        assert!(!s.progress_complete);
    }

    #[test]
    fn one_time_budget_progress_is_balance() {
        let b = budget(BudgetPeriodType::OneTime, 5_000);
        let txs = vec![
            linked(
                TransactionType::Allocation,
                7_000,
                local_midnight(2026, 0, 5),
            ),
            linked(TransactionType::Expense, 9_000, local_midnight(2026, 0, 6)),
        ];
        let s = compute_budget_stats(&b, &txs, local_midnight(2026, 0, 1));
        assert_eq!(s.balance, -2_000);
        assert!(s.overdrawn);
        assert_eq!(s.progress_percent, 0.0);
    }

    #[test]
    fn recurring_fills_accumulate_per_month() {
        let b = budget(BudgetPeriodType::Monthly, 1_000);
        let mut t = linked(
            TransactionType::Allocation,
            1_000,
            local_midnight(2026, 0, 1),
        );
        t.recurrence = Some(RecurrenceFrequency::Monthly);
        let s = compute_budget_stats(&b, &[t], local_midnight(2026, 2, 1));
        assert_eq!(s.balance, 3_000);
        assert_eq!(s.contributed, 1_000);
        assert!(s.progress_complete);
    }

    #[test]
    fn withdrawable_only_positive_balances() {
        let b = budget(BudgetPeriodType::Monthly, 0);
        let txs = vec![linked(
            TransactionType::Allocation,
            500,
            local_midnight(2026, 0, 2),
        )];
        let budgets = vec![b];
        assert_eq!(
            withdrawable_budgets(&budgets, &txs, local_midnight(2026, 0, 1)).len(),
            1
        );
        assert!(withdrawable_budgets(&budgets, &[], local_midnight(2026, 0, 1)).is_empty());
    }
}
