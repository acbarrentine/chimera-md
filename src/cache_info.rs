use std::time::SystemTime;
use crate::chimera_error::ChimeraError;

#[derive(Debug, PartialEq)]
pub struct Modtimes {
    md_modtime: SystemTime,
    hb_modtime: SystemTime,
    folder_modtime: SystemTime,
}

pub async fn get_modtime(path: &std::path::Path) -> Result<SystemTime, ChimeraError> {
    let md_metadata = tokio::fs::metadata(path).await?;
    Ok(md_metadata.modified()?)
}

impl Modtimes {
    pub async fn new(path: &str, hb_modtime: SystemTime) -> Self {
        let mut folder_modtime = SystemTime::UNIX_EPOCH;
        let path_buf = std::path::PathBuf::from(path);
        let parent_path = path_buf.parent();
        if let Some(parent_path) = parent_path {
            if let Ok(modtime) = get_modtime(parent_path).await {
                folder_modtime = modtime;
            }
        }
    
        let md_modtime = match get_modtime(path_buf.as_path()).await {
            Ok(modtime) => modtime,
            Err(_) => SystemTime::UNIX_EPOCH,
        };
    
        Modtimes {
            md_modtime,
            hb_modtime,
            folder_modtime,
        }
    }
}
