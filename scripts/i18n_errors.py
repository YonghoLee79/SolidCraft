"""Inventory Rust error literals (tree-sitter, not regex over source code).

Install scripts/requirements-i18n-errors.txt, then run with --add-missing or --check.
The JSON report records source locations. Its scope is interactive tool validation,
not backend, control-channel or scenario diagnostics.
This is catalog preparation; the runtime adapter preserves unmatched diagnostics.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
from tree_sitter import Language, Parser
import tree_sitter_rust

PARSER = Parser(Language(tree_sitter_rust.language()))
HELPERS = {"err", "bad", "invalid", "fail", "failed"}
WRAPPERS = {"into", "to_string", "to_owned", "clone"}


def text(node, data):
    return data[node.start_byte:node.end_byte].decode("utf-8")


def literal(node, data):
    s = text(node, data)
    if node.type == "raw_string_literal":
        start = s.index('"'); end = s.rindex('"')
        return s[start + 1:end]
    if node.type != "string_literal":
        return None
    s = re.sub(r"\\u\{([0-9a-fA-F_]+)\}", lambda m: chr(int(m[1].replace('_', ''), 16)), s)
    s = re.sub(r"\\\r?\n\s*", "", s)
    try:
        return json.loads(s.replace("\\0", "\\u0000"))
    except ValueError:
        return None


def normalize_format(source):
    """Rust formatting has already happened at display time; capture rendered values literally."""
    result = []; index = 0; auto = 0
    while index < len(source):
        if source[index:index + 2] in {"{{", "}}"}:
            result.append(source[index:index + 2]); index += 2; continue
        if source[index] == '{':
            end = source.find('}', index)
            if end < 0:
                return None
            field = source[index + 1:end].split(':', 1)[0].split('!', 1)[0]
            if not field:
                field = str(auto); auto += 1
            name = 'arg' + str(int(field) + 1) if field.isdigit() else field
            if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", name):
                return None
            result.append('{' + name + '}'); index = end + 1
        else:
            result.append(source[index]); index += 1
    return ''.join(result)


def message(node, data):
    value = literal(node, data)
    if value is not None:
        # Escape literal braces so they cannot be mistaken for substitutions.
        return value.replace('{', '{{').replace('}', '}}')
    if node.type == "macro_invocation":
        name = node.child_by_field_name("macro")
        if name is not None and text(name, data) in {"format", "format_args"}:
            tt = next((n for n in node.named_children if n.type == "token_tree"), None)
            if tt is not None and tt.named_children:
                value = literal(tt.named_children[0], data)
                return normalize_format(value) if value is not None else None
    if node.type == "call_expression":
        fn = node.child_by_field_name("function")
        if fn is not None and fn.type == "field_expression":
            method = fn.child_by_field_name("field")
            if method is not None and text(method, data) in WRAPPERS:
                return message(fn.child_by_field_name("value"), data)
    if node.type == "closure_expression":
        body = node.child_by_field_name("body")
        if body is not None:
            return message(body, data)
    return None


UI_FILES = {'dim_view', 'params_dialog', 'design_properties', 'bom_panel'}
INTERNAL_MESSAGES = {
    'Library response is missing',
    'Select an exact author placement before converting this plane',
    'Sketch author frame is invalid',
    '`joint` must be an id or a name',
    '`params` must be an object',
    '`study` must be an id or a name',
    '`values` must be a list',
    '`{arg1}` takes {arg2} value(s)',
    'not a Motion Study dialog',
    'not a move',
    'the Emboss author frame is degenerate',
    'unknown param `{key}` (Drive Joints takes joint, values)',
    'unknown param `{key}` (Motion Study takes study)',
    '{arg1} takes no params yet',
}
AXIS_ERRORS = {'the {axis} axis does not lie in the sketch plane', 'the revolve axis does not lie in the sketch plane'}


def in_scope(path, value):
    # Public interactive tools, not control/scenario APIs, renderer internals or import kernels.
    if value in INTERNAL_MESSAGES:
        return False
    return (path.parts[:3] == ('crates', 'ui-egui', 'src') and
            (path.stem.startswith(('dialogs', 'sketch')) or path.stem in UI_FILES)) or value in AXIS_ERRORS


def inventory(root, scoped=True):
    found = {}
    def record(value, path, node):
        if not value or not re.search(r"[A-Za-z]{2,}\s+[A-Za-z{`]|[A-Za-z]{2,}:\s", value):
            return
        if scoped and not in_scope(path, value):
            return
        found.setdefault(value, []).append({"file": path.as_posix(), "line": node.start_point.row + 1})
    for folder in ('crates', 'apps'):
        for path in sorted((root / folder).rglob('*.rs')):
            rel = path.relative_to(root)
            if any(part in {'tests', 'benches', 'examples', 'target'} for part in rel.parts) or 'test' in path.stem:
                continue
            data = path.read_bytes(); tree = PARSER.parse(data)
            def walk(parent):
                attrs = []
                for node in parent.named_children:
                    if node.type == 'attribute_item':
                        attrs.append(text(node, data))
                        # thiserror's format string, not the values/expressions following it.
                        if re.match(r'#\[error\s*\(', attrs[-1]):
                            def strings(n):
                                if n.type in {'string_literal', 'raw_string_literal'}:
                                    return [n]
                                return [s for c in n.named_children for s in strings(c)]
                            values = strings(node)
                            if values:
                                record(normalize_format(literal(values[0], data) or ''), rel, node)
                        continue
                    skip = any(re.search(r'\btest\b', a) for a in attrs); attrs = []
                    if skip:
                        continue
                    if node.type == 'call_expression':
                        fn = node.child_by_field_name('function'); args = node.child_by_field_name('arguments')
                        name = text(fn, data) if fn is not None else ''
                        simple = name.split('::')[-1]
                        method = fn.child_by_field_name('field') if fn is not None and fn.type == 'field_expression' else None
                        is_sink = simple in HELPERS | {'Err'} or ('Error::' in name) or (method is not None and text(method, data) in {'ok_or', 'ok_or_else'})
                        if is_sink and args is not None:
                            candidates = list(args.named_children)
                            if simple == 'bad' and len(candidates) > 1:
                                candidates = candidates[1:2]
                            for arg in candidates:
                                value = message(arg, data)
                                if value is not None:
                                    record(value, rel, arg)
                    walk(node)
            walk(tree.root_node)
    for value in found:
        found[value] = [dict(t) for t in sorted({tuple(x.items()) for x in found[value]})]
    return found


def key(value):
    words = re.sub(r'\{[^}]*\}', '', value)
    slug = '_'.join(re.findall('[a-z0-9]+', words.lower())[:8]) or 'message'
    return 'error.' + slug + '.' + hashlib.sha256(value.encode()).hexdigest()[:10]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--add-missing', action='store_true'); ap.add_argument('--check', action='store_true')
    ap.add_argument('--report', default='target/error-messages.json'); ap.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    args = ap.parse_args(); root = args.root
    path = root / 'assets/locales/en.json'; catalog = json.loads(path.read_text(encoding='utf8'))
    found = inventory(root); known = set(catalog.values()); missing = {value: locs for value, locs in found.items() if value not in known}
    report = {'messages': [{'id': next((k for k,v in catalog.items() if v == value), key(value)), 'english': value, 'locations': locs} for value, locs in sorted(found.items())], 'scope': 'Interactive tool and sketch validation, plus the two revolve axis diagnostics; excludes backend/control/scenario/import diagnostics'}
    dest = root / args.report; dest.parent.mkdir(parents=True, exist_ok=True); dest.write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n', encoding='utf8')
    if args.add_missing:
        for value in missing:
            catalog[key(value)] = value
        path.write_text(json.dumps(catalog, ensure_ascii=False, indent=2, sort_keys=True)+'\n', encoding='utf8')
    print(f'{len(found)} distinct diagnostic templates; {sum(map(len, found.values()))} source locations; {len(missing)} missing English entries')
    return int(args.check and bool(missing) and not args.add_missing)


if __name__ == '__main__':
    sys.exit(main())
