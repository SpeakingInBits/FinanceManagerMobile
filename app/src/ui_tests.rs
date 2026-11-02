//! UI tests: the real window and controller, run headless with Slint's testing backend. They use
//! the app the way a person does (tapping labelled buttons, typing into fields) and check what's
//! on screen. Elements are found by their accessible labels; a `Text`'s label is its text.

use std::path::Path;
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model};

use crate::controller::{self, Controller};
use crate::{App, AppWindow, DATA_FILE};

const SHEET: &str = "AppWindow::sheet";
const DIALOG: &str = "AppWindow::dialog-box";

/// The app over the data file in a folder, as it starts on a phone.
struct TestApp {
    ui: AppWindow,
    _controller: Rc<Controller>,
}

impl TestApp {
    fn open(dir: &Path) -> Self {
        let ui = AppWindow::new().expect("window should open");
        ui.window().set_size(LogicalSize::new(420.0, 860.0));
        ui.show().expect("window should show");
        let controller = controller::attach(&ui, dir.join(DATA_FILE));
        Self {
            ui,
            _controller: controller,
        }
    }

    /// Whether some element on screen has this label (for `Text`, this text).
    fn shows(&self, label: &str) -> bool {
        ElementHandle::find_by_accessible_label(&self.ui, label)
            .next()
            .is_some()
    }

    /// The page title in the top bar.
    fn title(&self) -> String {
        [
            "Dashboard",
            "Transactions",
            "Categories",
            "Budgets",
            "Charts",
        ][self.ui.global::<App>().get_page() as usize]
            .to_string()
    }

    /// A dashboard figure, such as "Income".
    fn stat(&self, label: &str) -> String {
        let stats = self.ui.global::<App>().get_stats();
        let value = stats
            .iter()
            .find(|s| s.label == label)
            .unwrap_or_else(|| panic!("no {label:?} figure"))
            .value
            .to_string();
        assert!(self.shows(&value), "{label} figure {value} isn't on screen");
        value
    }

    /// Taps the one button or tab labelled `label`, anywhere in the window.
    fn tap(&self, label: &str) {
        let matches: Vec<_> = ElementHandle::find_by_accessible_label(&self.ui, label)
            .filter(is_tappable)
            .collect();
        tap_only(matches, label, "the window");
    }

    /// Taps the one button labelled `label` inside the element with id `container`.
    fn tap_in(&self, container: &str, label: &str) {
        let wanted = label.to_string();
        let matches = self
            .element(container)
            .query_descendants()
            .match_predicate(move |e| e.accessible_label().is_some_and(|l| l == wanted))
            .find_all()
            .into_iter()
            .filter(is_tappable)
            .collect();
        tap_only(matches, label, container);
    }

    /// Types `text` into the text field labelled `label`, replacing what's there.
    fn type_into(&self, label: &str, text: &str) {
        let field = ElementHandle::find_by_accessible_label(&self.ui, label)
            .find(|e| e.accessible_role() == Some(AccessibleRole::TextInput))
            .unwrap_or_else(|| panic!("no text field labelled {label:?}"));
        field.set_accessible_value(text);
    }

    /// Picks `option` in the dropdown labelled `label`.
    fn choose(&self, label: &str, option: &str) {
        let select = ElementHandle::find_by_accessible_label(&self.ui, label)
            .find(|e| e.accessible_role() == Some(AccessibleRole::Combobox))
            .unwrap_or_else(|| panic!("no dropdown labelled {label:?}"));
        // Back to the first option, then forward until it's the one wanted.
        loop {
            let before = select.accessible_value();
            select.invoke_accessible_decrement_action();
            if select.accessible_value() == before {
                break;
            }
        }
        while select.accessible_value().as_deref() != Some(option) {
            let before = select.accessible_value();
            select.invoke_accessible_increment_action();
            assert_ne!(
                select.accessible_value(),
                before,
                "{label:?} has no option {option:?}"
            );
        }
    }

    /// Presses and releases a key, as the Android back button does.
    fn press(&self, key: Key) {
        let text = slint::SharedString::from(key);
        let window = self.ui.window();
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
    }

    fn sheet_open(&self) -> bool {
        self.element_exists(SHEET)
    }

    fn dialog_open(&self) -> bool {
        self.element_exists(DIALOG)
    }

    fn element(&self, id: &str) -> ElementHandle {
        ElementHandle::find_by_element_id(&self.ui, id)
            .next()
            .unwrap_or_else(|| panic!("{id} isn't open"))
    }

    fn element_exists(&self, id: &str) -> bool {
        ElementHandle::find_by_element_id(&self.ui, id)
            .next()
            .is_some()
    }

    /// Adds a one-off transaction for today from the Transactions page.
    fn add_transaction(&self, kind: &str, amount: &str) {
        self.tap("Transactions");
        self.tap("Add");
        self.tap_in(SHEET, kind);
        self.type_into("Amount", amount);
        self.tap_in(SHEET, "Add");
    }
}

fn is_tappable(e: &ElementHandle) -> bool {
    matches!(
        e.accessible_role(),
        Some(AccessibleRole::Button | AccessibleRole::Tab)
    )
}

fn tap_only(matches: Vec<ElementHandle>, label: &str, place: &str) {
    match matches.as_slice() {
        [one] => {
            assert_ne!(
                one.accessible_enabled(),
                Some(false),
                "{label:?} is disabled"
            );
            one.mock_single_click(PointerEventButton::Left);
        }
        [] => panic!("no {label:?} button in {place}"),
        _ => panic!("{} {label:?} buttons in {place}", matches.len()),
    }
}

/// Starts the testing backend for this test's thread and opens the app over an empty folder.
fn start() -> (TestApp, tempfile::TempDir) {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().expect("temp folder");
    (TestApp::open(dir.path()), dir)
}

#[test]
fn opens_on_an_empty_dashboard() {
    let (app, _dir) = start();
    assert_eq!(app.title(), "Dashboard");
    assert_eq!(app.stat("Income"), "$0.00");
    assert_eq!(app.stat("Net"), "$0.00");
    assert!(app.shows("No budgets yet"));
    assert!(!app.sheet_open());
}

#[test]
fn bottom_tabs_switch_pages() {
    let (app, _dir) = start();
    for page in [
        "Transactions",
        "Categories",
        "Budgets",
        "Charts",
        "Dashboard",
    ] {
        app.tap(page);
        assert_eq!(app.title(), page);
    }
    app.tap("Transactions");
    assert!(app.shows("No transactions this month"));
    app.tap("Categories");
    assert!(app.shows("No categories yet"));
}

#[test]
fn income_and_expenses_add_up_on_the_dashboard() {
    let (app, _dir) = start();
    app.add_transaction("Income", "2500");
    app.add_transaction("Expense", "42.50");
    assert!(!app.sheet_open());
    assert!(app.shows("+$2,500.00"));
    assert!(app.shows("-$42.50"));

    app.tap("Dashboard");
    assert_eq!(app.stat("Income"), "$2,500.00");
    assert_eq!(app.stat("One-time expenses"), "$42.50");
    assert_eq!(app.stat("Net"), "$2,457.50");
}

#[test]
fn deleting_a_transaction_asks_first() {
    let (app, _dir) = start();
    app.add_transaction("Expense", "12");

    app.tap("Delete");
    assert!(app.dialog_open());
    assert!(app.shows("Delete this transaction?"));
    app.tap_in(DIALOG, "Cancel");
    assert!(!app.dialog_open());
    assert!(app.shows("-$12.00"));

    app.tap("Delete");
    app.tap_in(DIALOG, "Delete");
    assert!(!app.dialog_open());
    assert!(app.shows("No transactions this month"));
}

#[test]
fn editing_a_transaction_changes_its_amount() {
    let (app, _dir) = start();
    app.add_transaction("Expense", "12");

    app.tap("Edit");
    assert!(app.shows("Edit transaction"));
    app.type_into("Amount", "15.25");
    app.tap_in(SHEET, "Save");
    assert!(app.shows("-$15.25"));
    assert!(!app.shows("-$12.00"));
}

#[test]
fn filling_a_budget_needs_a_budget_chosen() {
    let (app, _dir) = start();
    app.tap("Transactions");
    app.tap("Add");
    app.tap_in(SHEET, "Fill budget");
    app.type_into("Amount", "50");
    app.tap_in(SHEET, "Add");
    assert!(app.sheet_open(), "the form should stay open");
    assert!(app.shows("Select a budget."));
}

#[test]
fn a_budget_can_be_created_and_filled() {
    let (app, _dir) = start();
    app.add_transaction("Income", "1000");

    app.tap("Budgets");
    app.tap("Add");
    app.tap_in(SHEET, "Add");
    assert!(app.shows("Enter a name."), "a budget needs a name");
    app.type_into("Name", "Vacation");
    app.type_into("Monthly contribution target", "200");
    app.tap_in(SHEET, "Add");
    assert!(!app.sheet_open());
    assert!(app.shows("Vacation"));

    app.tap("Transactions");
    app.tap("Add");
    app.tap_in(SHEET, "Fill budget");
    app.type_into("Amount", "150");
    app.choose("Fill budget", "Vacation");
    app.tap_in(SHEET, "Add");
    assert!(!app.sheet_open());

    app.tap("Dashboard");
    assert_eq!(app.stat("Contrib. to budgets"), "$150.00");
    assert_eq!(app.stat("Net after allocations"), "$850.00");
}

#[test]
fn categories_can_be_added_and_used() {
    let (app, _dir) = start();
    app.tap("Categories");
    app.tap("Add");
    app.type_into("Name", "Groceries");
    app.tap_in(SHEET, "Add");
    assert!(app.shows("Groceries"));

    app.tap("Transactions");
    app.tap("Add");
    app.type_into("Amount", "30");
    app.choose("Category", "Groceries");
    app.tap_in(SHEET, "Add");
    assert!(app.shows("Groceries"));
    assert!(app.shows("-$30.00"));
}

#[test]
fn back_closes_the_sheet_and_the_dialog() {
    let (app, _dir) = start();
    app.tap("Transactions");
    app.tap("Add");
    assert!(app.sheet_open());
    app.press(Key::Back);
    assert!(!app.sheet_open());

    app.add_transaction("Expense", "5");
    app.tap("Delete");
    assert!(app.dialog_open());
    app.press(Key::Back);
    assert!(!app.dialog_open());
    assert!(app.shows("-$5.00"), "back cancels rather than deletes");
}

#[test]
fn data_is_kept_after_restarting() {
    let (app, dir) = start();
    app.add_transaction("Income", "321");
    drop(app);

    let app = TestApp::open(dir.path());
    assert_eq!(app.stat("Income"), "$321.00");
}
