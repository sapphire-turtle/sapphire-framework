//! The schema-driven settings pages: what the schema lists, and what a person has typed.

// The settings screen (task 5) hosts the pages and the banner.
#[allow(dead_code)]
mod banner;
pub(crate) mod model;
#[allow(dead_code)]
mod page;
#[allow(dead_code)]
mod widgets;

#[allow(unused_imports)]
pub(crate) use banner::RestartBanner;
#[allow(unused_imports)]
pub(crate) use page::ConfigPage;
