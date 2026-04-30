//! Shell function table for OmniShell.
//!
//! Stores user-defined shell functions (from `fname() { body }` syntax).
//! Accessed via shrs States system as a shared mutable resource.

use std::collections::HashMap;
use shrs_lang::ast;

/// A table of user-defined shell functions.
#[derive(Default)]
pub struct FunctionTable {
    /// Function name → AST body.
    functions: HashMap<String, ast::Command>,
}

impl FunctionTable {
    /// Create a new empty function table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Define or replace a function.
    pub fn define(&mut self, name: String, body: ast::Command) {
        self.functions.insert(name, body);
    }

    /// Look up a function by name.
    pub fn get(&self, name: &str) -> Option<&ast::Command> {
        self.functions.get(name)
    }

    /// Remove a function.
    pub fn remove(&mut self, name: &str) -> Option<ast::Command> {
        self.functions.remove(name)
    }

    /// List all function names.
    pub fn names(&self) -> Vec<&str> {
        self.functions.keys().map(|s| s.as_str()).collect()
    }

    /// Check if a function exists.
    pub fn contains(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }
}
