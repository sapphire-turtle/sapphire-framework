//! The schema-driven settings pages: what the schema lists, and what a person has typed.

mod banner;
pub(crate) mod model;
mod page;
pub(crate) mod widgets;

pub(crate) use banner::RestartBanner;
pub(crate) use page::ConfigPage;
