//! Error type shared by every layer; each variant maps to one CLI exit code.

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    External(String),
    #[error("{0}")]
    Usage(String),
}

pub type Result<T> = std::result::Result<T, AppError>;

impl AppError {
    pub fn code(&self) -> i32 {
        match self {
            AppError::Validation(_) => 1,
            AppError::NotFound(_) => 2,
            AppError::Config(_) => 3,
            AppError::External(_) => 4,
            AppError::Usage(_) => 64,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Validation(_) => "validation",
            AppError::NotFound(_) => "not_found",
            AppError::Config(_) => "config",
            AppError::External(_) => "external",
            AppError::Usage(_) => "usage",
        }
    }

    pub fn io(path: &Path, err: std::io::Error) -> Self {
        AppError::Config(format!("{}: {err}", path.display()))
    }

    /// A TOML parse error with the file path and a 1-based `line N, column M`.
    pub fn toml(path: &Path, src: &str, err: &toml::de::Error) -> Self {
        let msg = err
            .message()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        match err.span() {
            Some(span) => {
                let (line, col) = line_col(src, span.start);
                AppError::Config(format!(
                    "{}: invalid TOML at line {line}, column {col}: {msg}",
                    path.display()
                ))
            }
            None => AppError::Config(format!("{}: invalid TOML: {msg}", path.display())),
        }
    }
}

fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let before = &src[..offset.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_is_one_based() {
        assert_eq!(line_col("a = 1\nb = ", 6), (2, 1));
        assert_eq!(line_col("abc", 2), (1, 3));
    }
}
