//! Platform-independent core of Finance Tracker: data model, local persistence, and the
//! calculations behind the dashboard, budgets and charts. Ported from the TypeScript web app.

pub mod budget;
pub mod charts;
pub mod currency;
pub mod date;
pub mod geometry;
pub mod model;
pub mod recurrence;
pub mod store;
