use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::ast::{ImportDecl, ValueType};
use crate::diagnostics::Diagnostics;

#[derive(Clone, Debug, Deserialize)]
pub struct ModuleSpec {
    pub name: String,
    pub library: String,
    #[serde(default)]
    pub native_sources: Vec<String>,
    #[serde(default)]
    pub functions: Vec<ModuleFunctionSpec>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModuleFunctionSpec {
    pub name: String,
    #[serde(default)]
    pub overloads: Vec<ModuleFunctionOverloadSpec>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModuleFunctionOverloadSpec {
    pub args: Vec<String>,
    #[serde(rename = "return")]
    pub return_type: String,
    pub symbol: String,
}

#[derive(Clone, Debug)]
pub struct LoadedModule {
    pub spec: ModuleSpec,
    pub root_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ResolvedExternalCall {
    pub symbol: String,
    pub argument_types: Vec<ValueType>,
    pub return_type: ValueType,
}

#[derive(Clone, Debug, Default)]
pub struct ModuleRegistry {
    modules: HashMap<String, LoadedModule>,
}

impl ModuleRegistry {
    pub fn load_for_imports(
        imports: &[ImportDecl],
        search_roots: &[PathBuf],
        diagnostics: &mut Diagnostics,
    ) -> Self {
        let mut registry = Self::default();
        let mut seen = HashSet::new();

        for import in imports {
            if !seen.insert(import.module.clone()) {
                diagnostics.warning(
                    Some(import.span),
                    format!("Duplicate import of module '{}'", import.module),
                );
                continue;
            }

            match find_module_descriptor(&import.module, search_roots) {
                Some(path) => {
                    let content = match fs::read_to_string(&path) {
                        Ok(content) => content,
                        Err(error) => {
                            diagnostics.error(
                                Some(import.span),
                                format!("Failed to read {}: {}", path.display(), error),
                            );
                            continue;
                        }
                    };

                    let spec = match toml::from_str::<ModuleSpec>(&content) {
                        Ok(spec) => spec,
                        Err(error) => {
                            diagnostics.error(
                                Some(import.span),
                                format!("Invalid module.toml for '{}': {}", import.module, error),
                            );
                            continue;
                        }
                    };

                    if spec.name != import.module {
                        diagnostics.error(
                            Some(import.span),
                            format!(
                                "Import '{}' points to module '{}' in {}",
                                import.module,
                                spec.name,
                                path.display()
                            ),
                        );
                        continue;
                    }

                    let root_dir = path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| PathBuf::from("."));

                    registry
                        .modules
                        .insert(import.module.clone(), LoadedModule { spec, root_dir });
                }
                None => diagnostics.error(
                    Some(import.span),
                    format!(
                        "Module '{}' not found. Expected runtime/modules/{}/module.toml",
                        import.module, import.module
                    ),
                ),
            }
        }

        registry
    }

    pub fn contains(&self, module_name: &str) -> bool {
        self.modules.contains_key(module_name)
    }

    pub fn module(&self, module_name: &str) -> Option<&LoadedModule> {
        self.modules.get(module_name)
    }

    pub fn modules(&self) -> impl Iterator<Item = (&String, &LoadedModule)> {
        self.modules.iter()
    }

    pub fn resolve_call(
        &self,
        module_name: &str,
        function_name: &str,
        argument_types: &[ValueType],
    ) -> Option<ResolvedExternalCall> {
        let module = self.modules.get(module_name)?;

        let function = module
            .spec
            .functions
            .iter()
            .find(|candidate| candidate.name == function_name)?;

        for overload in &function.overloads {
            let parsed_args: Option<Vec<ValueType>> = overload
                .args
                .iter()
                .map(|name| ValueType::from_name(name.as_str()))
                .collect();
            let Some(parsed_args) = parsed_args else {
                continue;
            };

            if parsed_args != argument_types {
                continue;
            }

            let return_type = ValueType::from_name(overload.return_type.as_str())?;
            return Some(ResolvedExternalCall {
                symbol: overload.symbol.clone(),
                argument_types: parsed_args,
                return_type,
            });
        }

        None
    }

    pub fn expected_signatures(&self, module_name: &str, function_name: &str) -> Vec<String> {
        let mut signatures = Vec::new();
        let Some(module) = self.modules.get(module_name) else {
            return signatures;
        };

        let Some(function) = module
            .spec
            .functions
            .iter()
            .find(|candidate| candidate.name == function_name)
        else {
            return signatures;
        };

        for overload in &function.overloads {
            signatures.push(format!(
                "{}({}) -> {}",
                function_name,
                overload.args.join(", "),
                overload.return_type
            ));
        }

        signatures
    }
}

fn find_module_descriptor(module_name: &str, search_roots: &[PathBuf]) -> Option<PathBuf> {
    for root in search_roots {
        let candidate = root.join("modules").join(module_name).join("module.toml");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}
