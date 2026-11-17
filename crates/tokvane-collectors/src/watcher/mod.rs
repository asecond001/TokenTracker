pub mod coordinator;
pub mod tailer;

pub use coordinator::{discover_files_for_parser, CollectorCoordinator, ScanReport};
pub use tailer::{open_shared_readonly, tail_file_incremental, TailFileOutcome};
