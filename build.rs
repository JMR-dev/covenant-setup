use embed_manifest::embed_manifest;

fn main() {
    embed_manifest(embed_manifest::new_manifest("Comctl32"))
        .expect("unable to embed application manifest");
}
