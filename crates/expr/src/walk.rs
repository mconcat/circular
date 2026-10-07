use cel::common::ast::{EntryExpr, Expr, IdedEntryExpr, IdedExpr};

pub(crate) enum Structure<'a> {
    Leaf,
    Node(Vec<&'a IdedExpr>),
    Comprehension {
        iter_range: &'a IdedExpr,
        accu_init: &'a IdedExpr,
        loop_cond: &'a IdedExpr,
        loop_step: &'a IdedExpr,
        result: &'a IdedExpr,
        iter_var: &'a str,
        iter_var2: Option<&'a str>,
        accu_var: &'a str,
    },
}

pub(crate) fn structure(term: &IdedExpr) -> Structure<'_> {
    match &term.expr {
        Expr::Unspecified | Expr::Literal(_) | Expr::Ident(_) => Structure::Leaf,
        Expr::Select(select) => Structure::Node(vec![select.operand.as_ref()]),
        Expr::Call(call) => {
            let mut children =
                Vec::with_capacity(usize::from(call.target.is_some()) + call.args.len());
            children.extend(call.target.iter().map(Box::as_ref));
            children.extend(&call.args);
            Structure::Node(children)
        }
        Expr::List(list) => Structure::Node(list.elements.iter().collect()),
        Expr::Map(map) => Structure::Node(entry_children(&map.entries)),
        Expr::Struct(node) => Structure::Node(entry_children(&node.entries)),
        Expr::Comprehension(node) => Structure::Comprehension {
            iter_range: &node.iter_range,
            accu_init: &node.accu_init,
            loop_cond: &node.loop_cond,
            loop_step: &node.loop_step,
            result: &node.result,
            iter_var: &node.iter_var,
            iter_var2: node.iter_var2.as_deref(),
            accu_var: &node.accu_var,
        },
    }
}

fn entry_children(entries: &[IdedEntryExpr]) -> Vec<&IdedExpr> {
    let mut children = Vec::new();
    for entry in entries {
        match &entry.expr {
            EntryExpr::MapEntry(entry) => {
                children.push(&entry.key);
                children.push(&entry.value);
            }
            EntryExpr::StructField(field) => children.push(&field.value),
        }
    }
    children
}
