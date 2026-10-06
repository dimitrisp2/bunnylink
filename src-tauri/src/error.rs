// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("The vault is locked. Unlock it with your master password.")]
    VaultLocked,
    #[error("Wrong master password.")]
    WrongPassword,
    #[error("{0} not found.")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("SSH error: {0}")]
    Ssh(#[from] russh::Error),
    #[error("Key error: {0}")]
    Key(#[from] russh::keys::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
