use std::path::PathBuf;

#[derive(Debug)]
pub(in crate::app) enum Picked {
    Workspace(PathBuf),
    WorkspaceCreated(Result<PathBuf, String>),
    PdfIndexed(Result<PathBuf, String>),
    PdfChosen(PathBuf),
    #[cfg(feature = "dicom-preview")]
    DicomExternal(PathBuf),
    PageCover(PathBuf, PathBuf),
    BookCover(PathBuf, PathBuf),
    Book(PathBuf, Option<PathBuf>),
}
