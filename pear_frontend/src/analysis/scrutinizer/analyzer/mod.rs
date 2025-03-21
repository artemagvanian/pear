mod analyzer;
mod heuristics;
mod result;

pub use {analyzer::run, result::PurityAnalysisResult, result::ImpurityReason};
