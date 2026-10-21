"""Source inventory regression tests; no CAD build or display is needed."""
import tempfile
from pathlib import Path
import unittest
from i18n_errors import inventory, normalize_format, in_scope


class ErrorInventoryTests(unittest.TestCase):
    def scan(self, source):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); path = root / 'crates/demo/src/lib.rs'
            path.parent.mkdir(parents=True); path.write_text(source, encoding='utf8')
            return inventory(root, scoped=False)

    def test_scope_excludes_backend_automation_and_control_diagnostics(self):
        self.assertTrue(in_scope(Path('crates/ui-egui/src/dialogs.rs'), 'pick an axis'))
        self.assertTrue(in_scope(Path('crates/doc/src/eval.rs'), 'the {axis} axis does not lie in the sketch plane'))
        self.assertFalse(in_scope(Path('crates/kernel/src/boolean.rs'), 'invalid topology'))
        self.assertFalse(in_scope(Path('crates/ui-egui/src/control.rs'), 'path is required'))
        self.assertFalse(in_scope(Path('crates/ui-egui/src/scenario.rs'), 'no rect on screen'))
        self.assertFalse(in_scope(Path('crates/ui-egui/src/dialogs.rs'), '`params` must be an object'))

    def test_engine_and_named_axis_templates(self):
        found = self.scan('fn run() { return Err(DocError::Invalid(format!("the {axis} axis does not lie in the sketch plane"))); }')
        self.assertIn('the {axis} axis does not lie in the sketch plane', found)

    def test_parameter_names_are_not_collected_as_errors(self):
        found = self.scan('fn run() { return Err(bad("solid.revolve", "Select a sketch first")); }')
        self.assertEqual(list(found), ['Select a sketch first'])

    def test_no_test_log_or_comment_messages(self):
        found = self.scan('''// Err("A comment is not an error")
fn run() { log::error!("Log message only"); }
#[test] fn check() { return Err("A test message"); }
#[cfg(test)] mod tests { fn x() { return Err("Another test message"); } }
''')
        self.assertEqual(found, {})

    def test_ok_or_closures_and_raw_strings(self):
        found = self.scan('''fn run() { let a=x.ok_or("Missing active sketch")?; let b=x.ok_or_else(|| format!("invalid distance {n:.3}"))?; return Err(r#"Cannot read \"file\""#); }''')
        self.assertEqual(set(found), {'Missing active sketch', 'invalid distance {n}', 'Cannot read "file"'})

    def test_thiserror_templates(self):
        found = self.scan('''enum E { #[error("invalid input: {0}")] Invalid(String), #[error(transparent)] Other(Inner) }''')
        self.assertEqual(list(found), ['invalid input: {arg1}'])

    def test_formatting_preserves_repeats_and_escaped_braces(self):
        self.assertEqual(normalize_format('distance {:.3} ({0:?}) from {name} {{JSON}}'), 'distance {arg1} ({arg1}) from {name} {{JSON}}')

    def test_literals_escape_json_braces(self):
        found = self.scan(r'''fn run() { return Err("Expected {x: 1} as JSON"); }''')
        self.assertIn('Expected {{x: 1}} as JSON', found)

    def test_duplicate_messages_keep_source_locations(self):
        found = self.scan('fn x() { return Err("No active sketch"); }\nfn y() { return Err("No active sketch"); }')
        self.assertEqual([loc['line'] for loc in found['No active sketch']], [1, 2])


if __name__ == '__main__':
    unittest.main()
