use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ast::{
    AssignTarget, BodyDecl, CallTarget, Expr, FuncDecl, Item, Program, Stmt, VarDecl,
};
use crate::diagnostics::Diagnostics;

#[derive(Clone, Debug)]
pub struct ParsedSource {
    pub path: PathBuf,
    pub program: Program,
}

#[derive(Clone, Debug)]
struct SourceMeta {
    module_name: String,
    sanitized_module_name: String,
    aliases: Vec<String>,
}

#[derive(Clone)]
struct TransformCtx<'a> {
    source: &'a ParsedSource,
    module_name: &'a str,
    local_imports: &'a HashSet<String>,
    functions_by_module: &'a HashMap<String, HashMap<String, String>>,
}

pub fn merge_sources(sources: &[ParsedSource], diagnostics: &mut Diagnostics) -> Option<Program> {
    if sources.is_empty() {
        diagnostics.error(None, "No Checkbe input files were provided");
        return None;
    }

    let mut metas = Vec::with_capacity(sources.len());
    for source in sources {
        metas.push(build_source_meta(source));
    }

    let mut local_alias_map: HashMap<String, String> = HashMap::new();
    for meta in &metas {
        for alias in &meta.aliases {
            let key = normalize_module_name(alias);
            if key.is_empty() {
                continue;
            }
            if let Some(existing) = local_alias_map.get(&key) {
                if existing != &meta.module_name {
                    diagnostics.error(
                        None,
                        format!(
                            "Ambiguous local module alias '{}': '{}' and '{}'",
                            alias, existing, meta.module_name
                        ),
                    );
                }
            } else {
                local_alias_map.insert(key, meta.module_name.clone());
            }
        }
    }

    let mut functions_by_module: HashMap<String, HashMap<String, String>> = HashMap::new();
    for (index, source) in sources.iter().enumerate() {
        let module_name = &metas[index].module_name;
        let sanitized_module = &metas[index].sanitized_module_name;
        let mut functions = HashMap::new();
        for item in &source.program.body.items {
            let Item::Func(func_decl) = item else {
                continue;
            };

            let global_name = if index == 0 {
                func_decl.name.clone()
            } else {
                format!("{}__{}", sanitized_module, func_decl.name)
            };
            functions.insert(func_decl.name.clone(), global_name);
        }
        functions_by_module.insert(module_name.clone(), functions);
    }

    let mut runtime_imports = Vec::new();
    let mut runtime_import_seen = HashSet::new();
    let mut local_imports_by_source = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let current_module = &metas[index].module_name;
        let mut local_imports = HashSet::new();
        for import in &source.program.imports {
            if let Some(local_module) = resolve_local_module(&local_alias_map, &import.module) {
                // Preserve runtime imports when the alias resolves to the current source module.
                // This avoids collisions such as file "math.checkbe" + `import Math`.
                if local_module != *current_module {
                    local_imports.insert(local_module);
                    continue;
                }
            }
            if runtime_import_seen.insert(import.module.clone()) {
                runtime_imports.push(import.clone());
            }
        }
        local_imports_by_source.push(local_imports);
    }

    let mut merged_items = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let module_name = &metas[index].module_name;
        let context = TransformCtx {
            source,
            module_name,
            local_imports: &local_imports_by_source[index],
            functions_by_module: &functions_by_module,
        };
        for item in &source.program.body.items {
            merged_items.push(transform_item(item, &context, diagnostics));
        }
    }

    if diagnostics.has_errors() {
        return None;
    }

    let entry_body = &sources[0].program.body;
    Some(Program {
        imports: runtime_imports,
        body: BodyDecl {
            name: entry_body.name.clone(),
            items: merged_items,
            span: entry_body.span,
        },
    })
}

fn transform_item(item: &Item, ctx: &TransformCtx<'_>, diagnostics: &mut Diagnostics) -> Item {
    match item {
        Item::Capability(decl) => {
            let mut updated = decl.clone();
            updated.initializer = transform_expr(&decl.initializer, ctx, diagnostics);
            Item::Capability(updated)
        }
        Item::Right(decl) => {
            let mut updated = decl.clone();
            updated.initializer = transform_expr(&decl.initializer, ctx, diagnostics);
            Item::Right(updated)
        }
        Item::Var(var_decl) => Item::Var(transform_var_decl(var_decl, ctx, diagnostics)),
        Item::Func(func_decl) => Item::Func(transform_function_decl(func_decl, ctx, diagnostics)),
    }
}

fn transform_var_decl(
    var_decl: &VarDecl,
    ctx: &TransformCtx<'_>,
    diagnostics: &mut Diagnostics,
) -> VarDecl {
    let mut updated = var_decl.clone();
    updated.initializer = transform_expr(&var_decl.initializer, ctx, diagnostics);
    updated
}

fn transform_function_decl(
    func_decl: &FuncDecl,
    ctx: &TransformCtx<'_>,
    diagnostics: &mut Diagnostics,
) -> FuncDecl {
    let mut updated = func_decl.clone();
    updated.name = map_local_function_name(ctx.module_name, &func_decl.name, ctx);
    updated.body = func_decl
        .body
        .iter()
        .map(|stmt| transform_stmt(stmt, ctx, diagnostics))
        .collect();
    updated
}

fn transform_stmt(stmt: &Stmt, ctx: &TransformCtx<'_>, diagnostics: &mut Diagnostics) -> Stmt {
    match stmt {
        Stmt::VarDecl(var_decl) => Stmt::VarDecl(transform_var_decl(var_decl, ctx, diagnostics)),
        Stmt::Assign {
            target,
            value,
            span,
        } => Stmt::Assign {
            target: transform_assign_target(target, ctx, diagnostics),
            value: transform_expr(value, ctx, diagnostics),
            span: *span,
        },
        Stmt::If {
            condition,
            then_branch,
            else_branch,
            span,
        } => Stmt::If {
            condition: transform_expr(condition, ctx, diagnostics),
            then_branch: then_branch
                .iter()
                .map(|stmt| transform_stmt(stmt, ctx, diagnostics))
                .collect(),
            else_branch: else_branch
                .iter()
                .map(|stmt| transform_stmt(stmt, ctx, diagnostics))
                .collect(),
            span: *span,
        },
        Stmt::While {
            condition,
            body,
            span,
        } => Stmt::While {
            condition: transform_expr(condition, ctx, diagnostics),
            body: body
                .iter()
                .map(|stmt| transform_stmt(stmt, ctx, diagnostics))
                .collect(),
            span: *span,
        },
        Stmt::DoWhile {
            body,
            condition,
            span,
        } => Stmt::DoWhile {
            body: body
                .iter()
                .map(|stmt| transform_stmt(stmt, ctx, diagnostics))
                .collect(),
            condition: transform_expr(condition, ctx, diagnostics),
            span: *span,
        },
        Stmt::Expr { expr, span } => Stmt::Expr {
            expr: transform_expr(expr, ctx, diagnostics),
            span: *span,
        },
        Stmt::Block { statements, span } => Stmt::Block {
            statements: statements
                .iter()
                .map(|stmt| transform_stmt(stmt, ctx, diagnostics))
                .collect(),
            span: *span,
        },
        Stmt::Return { value, span } => Stmt::Return {
            value: value
                .as_ref()
                .map(|expr| transform_expr(expr, ctx, diagnostics)),
            span: *span,
        },
    }
}

fn transform_assign_target(
    target: &AssignTarget,
    ctx: &TransformCtx<'_>,
    diagnostics: &mut Diagnostics,
) -> AssignTarget {
    match target {
        AssignTarget::Identifier(name) => AssignTarget::Identifier(name.clone()),
        AssignTarget::Index { array, index } => AssignTarget::Index {
            array: transform_expr(array, ctx, diagnostics),
            index: transform_expr(index, ctx, diagnostics),
        },
    }
}

fn transform_expr(expr: &Expr, ctx: &TransformCtx<'_>, diagnostics: &mut Diagnostics) -> Expr {
    match expr {
        Expr::IntLiteral(_, _)
        | Expr::FloatLiteral(_, _)
        | Expr::StringLiteral(_, _)
        | Expr::BoolLiteral(_, _)
        | Expr::Identifier(_, _) => expr.clone(),
        Expr::ArrayLiteral(items, span) => Expr::ArrayLiteral(
            items
                .iter()
                .map(|item| transform_expr(item, ctx, diagnostics))
                .collect(),
            *span,
        ),
        Expr::Index { array, index, span } => Expr::Index {
            array: Box::new(transform_expr(array, ctx, diagnostics)),
            index: Box::new(transform_expr(index, ctx, diagnostics)),
            span: *span,
        },
        Expr::NewObject { kind, args, span } => Expr::NewObject {
            kind: kind.clone(),
            args: args
                .iter()
                .map(|arg| transform_expr(arg, ctx, diagnostics))
                .collect(),
            span: *span,
        },
        Expr::Call { callee, args, span } => {
            let transformed_args: Vec<Expr> = args
                .iter()
                .map(|arg| transform_expr(arg, ctx, diagnostics))
                .collect();
            let transformed_callee = transform_callee(callee, *span, ctx, diagnostics);
            Expr::Call {
                callee: transformed_callee,
                args: transformed_args,
                span: *span,
            }
        }
        Expr::Unary { op, expr, span } => Expr::Unary {
            op: *op,
            expr: Box::new(transform_expr(expr, ctx, diagnostics)),
            span: *span,
        },
        Expr::Binary {
            left,
            op,
            right,
            span,
        } => Expr::Binary {
            left: Box::new(transform_expr(left, ctx, diagnostics)),
            op: *op,
            right: Box::new(transform_expr(right, ctx, diagnostics)),
            span: *span,
        },
        Expr::Ternary {
            condition,
            then_expr,
            else_expr,
            span,
        } => Expr::Ternary {
            condition: Box::new(transform_expr(condition, ctx, diagnostics)),
            then_expr: Box::new(transform_expr(then_expr, ctx, diagnostics)),
            else_expr: Box::new(transform_expr(else_expr, ctx, diagnostics)),
            span: *span,
        },
    }
}

fn transform_callee(
    callee: &CallTarget,
    span: crate::span::Span,
    ctx: &TransformCtx<'_>,
    diagnostics: &mut Diagnostics,
) -> CallTarget {
    match callee {
        CallTarget::Name(name) => {
            if name == "Int" {
                return CallTarget::Name(name.clone());
            }

            if let Some(global_name) = lookup_local_function(ctx.module_name, name, ctx) {
                CallTarget::Name(global_name)
            } else {
                CallTarget::Name(name.clone())
            }
        }
        CallTarget::Qualified { module, name } => {
            if let Some(local_module) = resolve_local_module_from_ctx(ctx, module) {
                if !ctx.local_imports.contains(&local_module) {
                    return CallTarget::Qualified {
                        module: module.clone(),
                        name: name.clone(),
                    };
                }

                if let Some(global_name) = lookup_local_function(&local_module, name, ctx) {
                    CallTarget::Name(global_name)
                } else {
                    diagnostics.error(
                        None,
                        format!(
                            "{}:{}:{}: Module '{}' does not export function '{}'",
                            ctx.source.path.display(),
                            span.line,
                            span.column,
                            module,
                            name
                        ),
                    );
                    CallTarget::Name(format!(
                        "{}__{}",
                        sanitize_symbol(&local_module),
                        name
                    ))
                }
            } else {
                CallTarget::Qualified {
                    module: module.clone(),
                    name: name.clone(),
                }
            }
        }
    }
}

fn map_local_function_name(module_name: &str, local_name: &str, ctx: &TransformCtx<'_>) -> String {
    lookup_local_function(module_name, local_name, ctx).unwrap_or_else(|| local_name.to_string())
}

fn lookup_local_function(module_name: &str, local_name: &str, ctx: &TransformCtx<'_>) -> Option<String> {
    ctx.functions_by_module
        .get(module_name)
        .and_then(|functions| functions.get(local_name))
        .cloned()
}

fn build_source_meta(source: &ParsedSource) -> SourceMeta {
    let body_name = source.program.body.name.clone();
    let trimmed_body = trim_body_suffix(&body_name);
    let module_name = if trimmed_body.is_empty() {
        body_name.clone()
    } else {
        trimmed_body
    };

    let mut aliases = vec![module_name.clone(), body_name];
    if let Some(stem) = source.path.file_stem().and_then(|stem| stem.to_str()) {
        aliases.push(stem.to_string());
        aliases.push(to_pascal_case(stem));
    }
    aliases.sort();
    aliases.dedup();

    SourceMeta {
        sanitized_module_name: sanitize_symbol(&module_name),
        module_name,
        aliases,
    }
}

fn resolve_local_module(alias_map: &HashMap<String, String>, module_name: &str) -> Option<String> {
    let key = normalize_module_name(module_name);
    alias_map.get(&key).cloned()
}

fn resolve_local_module_from_ctx(ctx: &TransformCtx<'_>, module_name: &str) -> Option<String> {
    let key = normalize_module_name(module_name);
    for known_module in ctx.functions_by_module.keys() {
        if normalize_module_name(known_module) == key {
            return Some(known_module.clone());
        }
    }
    None
}

fn normalize_module_name(name: &str) -> String {
    name.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn sanitize_symbol(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "Module".to_string()
    } else {
        out
    }
}

fn trim_body_suffix(body_name: &str) -> String {
    if body_name.len() > 4 && body_name.ends_with("Body") {
        body_name[..body_name.len() - 4].to_string()
    } else {
        body_name.to_string()
    }
}

fn to_pascal_case(stem: &str) -> String {
    let mut out = String::new();
    let mut uppercase_next = true;
    for ch in stem.chars() {
        if ch == '_' || ch == '-' || ch == ' ' {
            uppercase_next = true;
            continue;
        }
        if uppercase_next {
            out.extend(ch.to_uppercase());
            uppercase_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}
