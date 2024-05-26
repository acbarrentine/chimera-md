use tokio::{fs, io::{stdout, AsyncWriteExt}};

#[tokio::main]
async fn main() {
    match fs::read_to_string("/Users/acbarrentine/Documents/MKScript/MKScript.md").await {
        Ok(md_content) => {
            let md = markdown::to_html(md_content.as_str());
            stdout().write_all(md.as_bytes()).await.unwrap();
        },
        Err(msg) => {
            eprintln!("Could not read source: {}", msg);
        }
    }
}
