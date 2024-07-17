use fs2::FileExt;
use serde::{Deserialize, Serialize};

pub trait JsonSerializable: Sized + Serialize + for<'de> Deserialize<'de> {
    fn to_file(&self, fname: &str) -> anyhow::Result<()> {
        use std::fs::File;
        use std::io::{BufWriter, Write};

        let file = File::create(fname)?;

        file.lock_exclusive()?;

        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, self)?;
        Ok(writer.flush()?)
    }

    fn try_from_file(fname: &str) -> anyhow::Result<Self> {
        let file = std::fs::File::open(fname)?;

        file.lock_shared()?;

        Ok(serde_json::from_reader::<_, Self>(file)?)
    }
}