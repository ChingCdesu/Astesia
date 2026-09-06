use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

pub(super) struct AtomicOutput {
    writer: BufWriter<NamedTempFile>,
    destination: PathBuf,
}

impl AtomicOutput {
    pub(super) fn new(destination: impl AsRef<Path>) -> io::Result<Self> {
        let destination = destination.as_ref().to_path_buf();
        let directory = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let temporary = tempfile::Builder::new()
            .prefix(".astesia-")
            .tempfile_in(directory)?;
        Ok(Self {
            writer: BufWriter::new(temporary),
            destination,
        })
    }

    pub(super) fn commit(mut self) -> io::Result<()> {
        self.writer.flush()?;
        self.writer.get_ref().as_file().sync_all()?;
        let temporary = self
            .writer
            .into_inner()
            .map_err(|error| error.into_error())?;
        temporary
            .persist(self.destination)
            .map_err(|error| error.error)?;
        Ok(())
    }
}

impl Write for AtomicOutput {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.writer.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl Seek for AtomicOutput {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.writer.seek(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_commit_replaces_existing_output_and_abandonment_cleans_up() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("export.csv");
        std::fs::write(&path, "original").unwrap();
        {
            let mut output = AtomicOutput::new(&path).unwrap();
            output.write_all(b"abandoned").unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        let mut output = AtomicOutput::new(&path).unwrap();
        output.write_all(b"complete").unwrap();
        output.commit().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "complete");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
