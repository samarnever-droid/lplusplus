use std::fmt::{Debug, Write as _};

use crate::{Arena, ArenaId, HirPackage, Symbol};

#[must_use]
pub fn hir_snapshot(package: &HirPackage) -> String {
    let mut output = String::new();
    writeln!(output, "hir-v1").expect("writing to a String cannot fail");
    writeln!(
        output,
        "counts modules={} definitions={} symbols={} origins={} scopes={} locals={} type_refs={} type_parameters={} fields={} variants={} expressions={} statements={} bodies={} match_arms={} items={}",
        package.modules.len(),
        package.names.definitions.len(),
        package.names.symbols.len(),
        package.origins.len(),
        package.scopes.len(),
        package.locals.len(),
        package.type_refs.len(),
        package.type_parameters.len(),
        package.fields.len(),
        package.variants.len(),
        package.expressions.len(),
        package.statements.len(),
        package.bodies.len(),
        package.match_arms.len(),
        package.items.len(),
    )
    .expect("writing to a String cannot fail");

    writeln!(output, "symbols").expect("writing to a String cannot fail");
    for raw in 0..package.names.symbols.len() {
        let symbol = Symbol::from_raw(u32::try_from(raw).expect("symbol count fits its ID"));
        let spelling = package
            .names
            .symbols
            .resolve(symbol)
            .expect("allocated symbols retain their spelling");
        writeln!(output, "  {symbol:?} {spelling:?}").expect("writing to a String cannot fail");
    }

    write_arena(&mut output, "definitions", &package.names.definitions);
    writeln!(output, "module_scopes").expect("writing to a String cannot fail");
    for scope in &package.names.modules {
        writeln!(output, "  module {:?}", scope.module).expect("writing to a String cannot fail");
        for (symbol, definition) in scope.definitions() {
            writeln!(output, "    definition {symbol:?} -> {definition:?}")
                .expect("writing to a String cannot fail");
        }
        for (symbol, target) in scope.imports() {
            writeln!(output, "    import {symbol:?} -> {target:?}")
                .expect("writing to a String cannot fail");
        }
    }

    writeln!(output, "modules").expect("writing to a String cannot fail");
    for (index, module) in package.modules.iter().enumerate() {
        writeln!(output, "  {index:04} {module:?}").expect("writing to a String cannot fail");
    }
    write_arena(&mut output, "origins", &package.origins);
    write_arena(&mut output, "scopes", &package.scopes);
    write_arena(&mut output, "locals", &package.locals);
    write_arena(&mut output, "type_refs", &package.type_refs);
    write_arena(&mut output, "type_parameters", &package.type_parameters);
    write_arena(&mut output, "fields", &package.fields);
    write_arena(&mut output, "variants", &package.variants);
    write_arena(&mut output, "expressions", &package.expressions);
    write_arena(&mut output, "statements", &package.statements);
    write_arena(&mut output, "bodies", &package.bodies);
    write_arena(&mut output, "match_arms", &package.match_arms);
    write_arena(&mut output, "items", &package.items);

    writeln!(output, "lists").expect("writing to a String cannot fail");
    write_list(&mut output, "symbols", package.lists.symbols.as_slice());
    write_list(&mut output, "type_refs", package.lists.type_refs.as_slice());
    write_list(
        &mut output,
        "type_parameters",
        package.lists.type_parameters.as_slice(),
    );
    write_list(&mut output, "locals", package.lists.locals.as_slice());
    write_list(
        &mut output,
        "expressions",
        package.lists.expressions.as_slice(),
    );
    write_list(
        &mut output,
        "statements",
        package.lists.statements.as_slice(),
    );
    write_list(&mut output, "fields", package.lists.fields.as_slice());
    write_list(&mut output, "variants", package.lists.variants.as_slice());
    write_list(&mut output, "items", package.lists.items.as_slice());
    write_list(
        &mut output,
        "match_arms",
        package.lists.match_arms.as_slice(),
    );
    output
}

fn write_arena<I, T>(output: &mut String, label: &str, arena: &Arena<I, T>)
where
    I: ArenaId + Debug,
    T: Debug,
{
    writeln!(output, "{label}").expect("writing to a String cannot fail");
    for (id, value) in arena.enumerate() {
        writeln!(output, "  {id:?} {value:?}").expect("writing to a String cannot fail");
    }
}

fn write_list<T: Debug>(output: &mut String, label: &str, values: &[T]) {
    writeln!(output, "  {label} {values:?}").expect("writing to a String cannot fail");
}
