use std::collections::HashMap;

use crate::ast::{
    AssignTarget, BinaryOp, CallTarget, CapabilityDecl, Expr, FuncDecl, Item, Program, RightDecl,
    Stmt, UnaryOp, ValueType, VarDecl,
};
use crate::diagnostics::Diagnostics;
use crate::module_system::ModuleRegistry;
use crate::parser::parse_expression_fragment;
use crate::span::Span;
use crate::string_interp::{parse_segments, Segment};

#[derive(Clone, Copy, Debug)]
enum Operation {
    Read,
    Write,
}

#[derive(Clone, Debug)]
pub struct PermissionSet {
    pub raw: Vec<String>,
    allow_read: bool,
    allow_write: bool,
    allow_all: bool,
    deny_read: bool,
    deny_write: bool,
    deny_all: bool,
}

impl PermissionSet {
    fn from_atoms(atoms: Vec<String>) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut set = Self {
            raw: atoms.clone(),
            allow_read: false,
            allow_write: false,
            allow_all: false,
            deny_read: false,
            deny_write: false,
            deny_all: false,
        };

        for atom in atoms {
            let lowered = atom.to_ascii_lowercase();
            let deny = lowered.contains("forbid")
                || lowered.contains("deny")
                || lowered.contains("no_")
                || lowered.starts_with('!');
            let mut recognized = false;

            if lowered.contains("all") || lowered.contains("full") {
                recognized = true;
                if deny {
                    set.deny_all = true;
                } else {
                    set.allow_all = true;
                }
            }

            if lowered.contains("read") {
                recognized = true;
                if deny {
                    set.deny_read = true;
                } else {
                    set.allow_read = true;
                }
            }

            if lowered.contains("write") {
                recognized = true;
                if deny {
                    set.deny_write = true;
                } else {
                    set.allow_write = true;
                }
            }

            if !recognized {
                warnings.push(format!(
                    "Permission '{}' does not affect read/write/all and will be ignored",
                    atom
                ));
            }
        }

        if set.allow_read && set.deny_read {
            warnings.push("Conflicting permissions: both allow read and deny read".to_string());
        }
        if set.allow_write && set.deny_write {
            warnings.push("Conflicting permissions: both allow write and deny write".to_string());
        }

        (set, warnings)
    }

    fn allows(&self, operation: Operation) -> bool {
        if self.deny_all {
            return false;
        }

        match operation {
            Operation::Read => {
                if self.deny_read {
                    return false;
                }
                self.allow_all || self.allow_read
            }
            Operation::Write => {
                if self.deny_write {
                    return false;
                }
                self.allow_all || self.allow_write
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct VariableInfo {
    pub ty: ValueType,
    pub entitlement: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FunctionInfo {
    pub params: Vec<ValueType>,
    pub return_type: ValueType,
}

#[derive(Clone, Debug)]
pub struct SemanticModel {
    pub globals: HashMap<String, VariableInfo>,
    pub functions: HashMap<String, FunctionInfo>,
    pub modules: ModuleRegistry,
}

pub fn analyze(
    program: &Program,
    modules: ModuleRegistry,
    diagnostics: &mut Diagnostics,
) -> Option<SemanticModel> {
    let mut analyzer = SemanticAnalyzer::new(program, modules, diagnostics);
    analyzer.analyze_program()
}

struct SemanticAnalyzer<'a> {
    program: &'a Program,
    diagnostics: &'a mut Diagnostics,
    capabilities: HashMap<String, PermissionSet>,
    rights: HashMap<String, PermissionSet>,
    globals: HashMap<String, VariableInfo>,
    functions: HashMap<String, FunctionInfo>,
    modules: ModuleRegistry,
}

impl<'a> SemanticAnalyzer<'a> {
    fn new(
        program: &'a Program,
        modules: ModuleRegistry,
        diagnostics: &'a mut Diagnostics,
    ) -> Self {
        Self {
            program,
            diagnostics,
            capabilities: HashMap::new(),
            rights: HashMap::new(),
            globals: HashMap::new(),
            functions: HashMap::new(),
            modules,
        }
    }

    fn analyze_program(&mut self) -> Option<SemanticModel> {
        for item in &self.program.body.items {
            match item {
                Item::Capability(decl) => self.collect_capability(decl),
                Item::Right(decl) => self.collect_right(decl),
                _ => {}
            }
        }

        for item in &self.program.body.items {
            match item {
                Item::Var(decl) => self.collect_global_var_signature(decl),
                Item::Func(decl) => self.collect_function_signature(decl),
                _ => {}
            }
        }

        if !self.functions.contains_key("main") {
            self.diagnostics
                .error(None, "Required function main() is missing")
        }

        for item in &self.program.body.items {
            if let Item::Var(decl) = item {
                self.analyze_global_var_initializer(decl);
            }
        }

        for item in &self.program.body.items {
            if let Item::Func(func) = item {
                self.analyze_function(func);
            }
        }

        if self.diagnostics.has_errors() {
            return None;
        }

        Some(SemanticModel {
            globals: self.globals.clone(),
            functions: self.functions.clone(),
            modules: self.modules.clone(),
        })
    }

    fn collect_capability(&mut self, decl: &CapabilityDecl) {
        if self.capabilities.contains_key(&decl.name) {
            self.diagnostics.error(
                Some(decl.span),
                format!("Capability '{}' is already declared", decl.name),
            );
            return;
        }

        let Some(atoms) = self.extract_permission_atoms(&decl.initializer, "Capability") else {
            return;
        };

        let (set, warnings) = PermissionSet::from_atoms(atoms);
        for warning in warnings {
            self.diagnostics.warning(Some(decl.span), warning);
        }

        self.capabilities.insert(decl.name.clone(), set);
    }

    fn collect_right(&mut self, decl: &RightDecl) {
        if self.rights.contains_key(&decl.name) {
            self.diagnostics.error(
                Some(decl.span),
                format!("Right '{}' is already declared", decl.name),
            );
            return;
        }

        let Some(atoms) = self.extract_permission_atoms(&decl.initializer, "Right") else {
            return;
        };

        let (set, warnings) = PermissionSet::from_atoms(atoms);
        for warning in warnings {
            self.diagnostics.warning(Some(decl.span), warning);
        }

        self.rights.insert(decl.name.clone(), set);
    }

    fn extract_permission_atoms(
        &mut self,
        initializer: &Expr,
        expected_kind: &str,
    ) -> Option<Vec<String>> {
        let Expr::NewObject { kind, args, span } = initializer else {
            self.diagnostics.error(
                Some(initializer.span()),
                format!(
                    "Expected constructor of form new {}(...), got a different expression",
                    expected_kind
                ),
            );
            return None;
        };

        if kind != expected_kind {
            self.diagnostics.warning(
                Some(*span),
                format!(
                    "Constructor '{}' is used for {}, expected '{}'",
                    kind, expected_kind, expected_kind
                ),
            );
        }

        let mut atoms = Vec::new();
        for arg in args {
            match arg {
                Expr::Identifier(name, _) | Expr::StringLiteral(name, _) => {
                    atoms.push(name.clone())
                }
                _ => {
                    self.diagnostics.error(
                        Some(arg.span()),
                        "Only identifiers or string literals are allowed in new Capability/new Right",
                    );
                    return None;
                }
            }
        }

        if atoms.is_empty() {
            self.diagnostics.warning(
                Some(*span),
                format!(
                    "{} declared without permissions (access denied by default)",
                    expected_kind
                ),
            );
        }

        Some(atoms)
    }

    fn collect_global_var_signature(&mut self, decl: &VarDecl) {
        if self.globals.contains_key(&decl.name) {
            self.diagnostics.error(
                Some(decl.span),
                format!("Variable '{}' is already declared", decl.name),
            );
            return;
        }

        if let Some(capability) = &decl.entitlement {
            if !self.capabilities.contains_key(capability) {
                self.diagnostics.error(
                    Some(decl.span),
                    format!("Capability '{}' not found", capability),
                );
            }
        }

        self.globals.insert(
            decl.name.clone(),
            VariableInfo {
                ty: ValueType::Void,
                entitlement: decl.entitlement.clone(),
                span: decl.span,
            },
        );
    }

    fn collect_function_signature(&mut self, decl: &FuncDecl) {
        if self.functions.contains_key(&decl.name) {
            self.diagnostics.error(
                Some(decl.span),
                format!("Function '{}' is already declared", decl.name),
            );
            return;
        }

        if decl.name == "main" {
            if decl.right.is_some() {
                self.diagnostics.warning(
                    Some(decl.span),
                    "Rights on main are ignored; main always has full rights",
                );
            }
            match decl.params.as_slice() {
                [] => {}
                [argc, argv]
                    if argc.ty == ValueType::Int
                        && argv.ty == ValueType::Array(Box::new(ValueType::String)) => {}
                _ => self.diagnostics.error(
                    Some(decl.span),
                    "main() must be either main() or main(argc: int, argv: string[])",
                ),
            }
            if decl.return_type != ValueType::Void {
                self.diagnostics
                    .error(Some(decl.span), "main() must have return type void");
            }
        } else if let Some(right_name) = &decl.right {
            if !self.rights.contains_key(right_name) {
                self.diagnostics
                    .error(Some(decl.span), format!("Right '{}' not found", right_name));
            }
        }

        let mut params = Vec::with_capacity(decl.params.len());
        for param in &decl.params {
            params.push(param.ty.clone());
        }

        self.functions.insert(
            decl.name.clone(),
            FunctionInfo {
                params,
                return_type: decl.return_type.clone(),
            },
        );
    }

    fn analyze_global_var_initializer(&mut self, decl: &VarDecl) {
        let mut scope = HashMap::new();
        for (name, info) in &self.globals {
            if name != &decl.name && info.ty != ValueType::Void {
                scope.insert(name.clone(), info.clone());
            }
        }

        let Some(expr_type) = self.analyze_expr(&decl.initializer, &scope, None) else {
            return;
        };

        let resolved_type = if let Some(explicit_type) = &decl.explicit_type {
            if !is_assignable(explicit_type, &expr_type) {
                self.diagnostics.error(
                    Some(decl.span),
                    format!(
                        "Initializer type '{}' is not compatible with declared type '{}'",
                        expr_type.as_str(),
                        explicit_type.as_str()
                    ),
                );
                explicit_type.clone()
            } else {
                explicit_type.clone()
            }
        } else {
            expr_type
        };

        if let Some(info) = self.globals.get_mut(&decl.name) {
            info.ty = resolved_type;
        }
    }

    fn analyze_function(&mut self, decl: &FuncDecl) {
        let right: Option<PermissionSet> = if decl.name == "main" {
            None
        } else {
            decl.right
                .as_ref()
                .and_then(|right_name| self.rights.get(right_name))
                .cloned()
        };

        let mut locals = HashMap::new();
        for param in &decl.params {
            if locals.contains_key(&param.name) {
                self.diagnostics.error(
                    Some(param.span),
                    format!("Duplicate parameter '{}'", param.name),
                );
                continue;
            }

            locals.insert(
                param.name.clone(),
                VariableInfo {
                    ty: param.ty.clone(),
                    entitlement: None,
                    span: param.span,
                },
            );
        }

        let mut has_return = false;
        for statement in &decl.body {
            self.analyze_statement(
                statement,
                &mut locals,
                right.as_ref(),
                &decl.return_type,
                &mut has_return,
            );
        }

        if decl.return_type != ValueType::Void && !has_return {
            self.diagnostics.error(
                Some(decl.span),
                format!(
                    "Function '{}' must return '{}' on at least one path",
                    decl.name,
                    decl.return_type.as_str()
                ),
            );
        }
    }
}
