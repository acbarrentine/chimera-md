//use markdown::ParseOptions;
use tokio::{fs, io::{stdout, AsyncWriteExt}};

#[tokio::main]
async fn main() {
    match fs::read_to_string("examples/documentation.md").await {
        Ok(md_content) => {
            // let blocks = markdown::to_mdast(md_content.as_str(), &ParseOptions::default());
            // match blocks {
            //     Ok(node) => {
            //         stdout().write_all(node.to_string().as_bytes()).await.unwrap();
            //     },
            //     Err(e) => {
            //         eprintln!("Failed converting to markdown?: {e}");
            //     }
            // }
            let md = markdown::to_html(md_content.as_str());
            stdout().write_all(md.as_bytes()).await.unwrap();
        },
        Err(msg) => {
            eprintln!("Could not read source: {}", msg);
        }
    }
}
