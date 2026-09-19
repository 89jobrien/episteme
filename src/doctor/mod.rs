//! Read-only dependency diagnostics.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// One required external executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredTool {
    /// Human-readable tool name.
    pub name: String,
    /// Explicit executable path.
    pub path: PathBuf,
}

/// Availability result for one dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCheck {
    /// Human-readable tool name.
    pub name: String,
    /// Configured executable path.
    pub path: PathBuf,
    /// Whether the path names an executable regular file.
    pub available: bool,
}

/// Read-only dependency health report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    /// Individual dependency checks.
    pub checks: Vec<ToolCheck>,
}

impl DoctorReport {
    /// Returns whether every dependency is available.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.checks.iter().all(|check| check.available)
    }
}

/// Performs read-only dependency checks.
#[derive(Debug, Default, Clone, Copy)]
pub struct Doctor;

impl Doctor {
    /// Checks explicit executable paths without invoking them.
    #[must_use]
    pub fn check_tools(tools: &[RequiredTool]) -> DoctorReport {
        let checks = tools
            .iter()
            .map(|tool| {
                let available = fs::metadata(&tool.path).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                });
                ToolCheck {
                    name: tool.name.clone(),
                    path: tool.path.clone(),
                    available,
                }
            })
            .collect();
        DoctorReport { checks }
    }
}
