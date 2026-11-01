//! Projection of recurring transactions into months.

use crate::date::{days_in_month, local_midnight, month_bounds, shift_month, start_of_month, ymd};
use crate::model::{Cents, Millis, RecurrenceFrequency, Transaction};

/// Converts a per-occurrence amount to its monthly-equivalent (yearly / 12, rounded to cents).
pub fn monthly_equivalent_amount(amount: Cents, recurrence: Option<RecurrenceFrequency>) -> Cents {
    match recurrence {
        Some(RecurrenceFrequency::Yearly) => (amount as f64 / 12.0).round() as Cents,
        _ => amount,
    }
}

/// A transaction as it should be shown/summed for a specific month.
#[derive(Clone, Copy, Debug)]
pub struct MonthlyOccurrence<'a> {
    pub transaction: &'a Transaction,
    pub display_date: Millis,
    pub display_amount: Cents,
}

/// Re-dates `anchor` into `month_start`'s month, clamping the day-of-month to the month's length.
pub fn project_day_into_month(anchor: Millis, month_start: Millis) -> Millis {
    let (_, _, anchor_day) = ymd(anchor);
    let (y, m, _) = ymd(month_start);
    let day = anchor_day.min(days_in_month(y, m as i32));
    local_midnight(y, m as i32, day)
}

/// Expands a transaction into every occurrence realized up to and including `month_start`'s
/// month. A one-off yields its single dated occurrence (wherever it falls, even past
/// `month_start`); a recurring transaction yields one monthly-equivalent occurrence per month from
/// its anchor month through `month_start`, none if it starts after `month_start`, and none in or
/// after its `recurrence_end` month when set.
pub fn occurrences_through_month(
    t: &Transaction,
    month_start: Millis,
) -> Vec<MonthlyOccurrence<'_>> {
    if t.recurrence.is_none() {
        return vec![MonthlyOccurrence {
            transaction: t,
            display_date: t.date,
            display_amount: t.amount,
        }];
    }
    let display_amount = monthly_equivalent_amount(t.amount, t.recurrence);
    let mut limit = start_of_month(month_start);
    if let Some(end) = t.recurrence_end {
        limit = limit.min(shift_month(end, -1));
    }
    let mut out = Vec::new();
    let mut m = start_of_month(t.date);
    while m <= limit {
        out.push(MonthlyOccurrence {
            transaction: t,
            display_date: project_day_into_month(t.date, m),
            display_amount,
        });
        m = shift_month(m, 1);
    }
    out
}

/// Projects recurring transactions into `month_start`'s month as monthly-equivalent occurrences
/// (starting the month they were created in and stopping before their `recurrence_end` month,
/// when set), and includes one-off transactions dated within it.
pub fn occurrences_for_month(
    transactions: &[Transaction],
    month_start: Millis,
) -> Vec<MonthlyOccurrence<'_>> {
    let (range_start, range_end) = month_bounds(month_start);
    let mut out = Vec::new();
    for t in transactions {
        if t.recurrence.is_none() {
            if t.date >= range_start && t.date <= range_end {
                out.push(MonthlyOccurrence {
                    transaction: t,
                    display_date: t.date,
                    display_amount: t.amount,
                });
            }
            continue;
        }
        if start_of_month(t.date) > month_start {
            continue;
        }
        if t.recurrence_end.is_some_and(|end| month_start >= end) {
            continue;
        }
        out.push(MonthlyOccurrence {
            transaction: t,
            display_date: project_day_into_month(t.date, month_start),
            display_amount: monthly_equivalent_amount(t.amount, t.recurrence),
        });
    }
    out
}

/// Month-scoped copies of transactions with each amount replaced by its display amount, the
/// shape the dashboard and charts aggregate over.
pub fn month_view(transactions: &[Transaction], month_start: Millis) -> Vec<Transaction> {
    occurrences_for_month(transactions, month_start)
        .into_iter()
        .map(|o| Transaction {
            amount: o.display_amount,
            ..o.transaction.clone()
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::TransactionType;

    pub fn tx(kind: TransactionType, amount: Cents, date: Millis) -> Transaction {
        Transaction {
            id: format!("t{amount}-{date}"),
            kind,
            amount,
            date,
            category_id: None,
            subcategory_id: None,
            budget_id: None,
            note: String::new(),
            recurrence: None,
            recurrence_end: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn yearly_is_divided_by_twelve() {
        assert_eq!(
            monthly_equivalent_amount(1200, Some(RecurrenceFrequency::Yearly)),
            100
        );
        assert_eq!(
            monthly_equivalent_amount(1000, Some(RecurrenceFrequency::Yearly)),
            83
        );
        assert_eq!(
            monthly_equivalent_amount(1000, Some(RecurrenceFrequency::Monthly)),
            1000
        );
    }

    #[test]
    fn projects_day_clamped_to_month_length() {
        let anchor = local_midnight(2026, 0, 31);
        let feb = local_midnight(2026, 1, 1);
        assert_eq!(
            project_day_into_month(anchor, feb),
            local_midnight(2026, 1, 28)
        );
    }

    #[test]
    fn recurring_appears_from_anchor_until_end() {
        let mut t = tx(TransactionType::Expense, 500, local_midnight(2026, 0, 15));
        t.recurrence = Some(RecurrenceFrequency::Monthly);
        t.recurrence_end = Some(local_midnight(2026, 3, 1));
        let list = vec![t];
        assert!(occurrences_for_month(&list, local_midnight(2025, 11, 1)).is_empty());
        assert_eq!(
            occurrences_for_month(&list, local_midnight(2026, 2, 1)).len(),
            1
        );
        assert!(occurrences_for_month(&list, local_midnight(2026, 3, 1)).is_empty());
        let through = occurrences_through_month(&list[0], local_midnight(2026, 5, 1));
        assert_eq!(through.len(), 3);
        assert_eq!(through[2].display_date, local_midnight(2026, 2, 15));
    }

    #[test]
    fn one_offs_only_in_their_month() {
        let list = vec![tx(
            TransactionType::Income,
            100,
            local_midnight(2026, 4, 31),
        )];
        assert_eq!(
            occurrences_for_month(&list, local_midnight(2026, 4, 1)).len(),
            1
        );
        assert!(occurrences_for_month(&list, local_midnight(2026, 5, 1)).is_empty());
        // occurrences_through_month always yields a one-off, wherever it falls.
        assert_eq!(
            occurrences_through_month(&list[0], local_midnight(2020, 0, 1)).len(),
            1
        );
    }
}
