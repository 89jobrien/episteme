//! Local-first knowledge ingestion and intelligence workflows.

pub mod adapters;
#[allow(clippy::all, clippy::pedantic)]
pub mod baml_client;
pub mod classification;
pub mod config;
pub mod doctor;
pub mod domain;
pub mod ingest;
pub mod intelligence;
pub mod ports;
pub mod stage;
pub mod watch;
