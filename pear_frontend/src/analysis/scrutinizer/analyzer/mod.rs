mod analyzer;
mod heuristics;
mod result;

pub use {
    analyzer::ImportantArgs, analyzer::ScrutinizerAnalysis, result::ImpurityReason,
    result::PurityAnalysisResult,
};
