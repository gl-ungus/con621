use serde::Deserialize;

const BASE_URL: &str = "https://e621.net";
const USER_AGENT: &str = "con621/0.1.0 (console client)";

/// Shared blocking HTTP client, built once and reused so connections to e621
/// are pooled/kept alive across requests. e621 requires a User-Agent on every
/// request. ponytail: OnceLock over a custom singleton; std covers it.
pub fn client() -> Result<&'static reqwest::blocking::Client, String> {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let c = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(CLIENT.get_or_init(|| c))
}

/// GET a URL and return the raw bytes.
pub fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let bytes = client()?
        .get(url)
        .send()
        .map_err(|e| e.to_string())?
        .bytes()
        .map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

#[derive(Debug, Deserialize, Clone)]
pub struct Post {
    pub id: u64,
    pub score: Score,
    pub fav_count: u32,
    pub rating: String,
    pub file: FileInfo,
    pub preview: PreviewInfo,
    #[serde(default)]
    pub sample: SampleInfo,
    pub tags: Tags,
    pub description: String,
    pub sources: Vec<String>,
    pub created_at: Option<String>,
}

impl Post {
    /// True if this post is an animated/video format we play back as frames.
    pub fn is_video(&self) -> bool {
        matches!(self.file.ext.as_deref(), Some("webm" | "mp4" | "gif"))
    }

    /// Best still-image URL to use for a (scaled-up) preview: prefer the larger
    /// `sample` image, then the full file (if it's an image), then the thumb.
    pub fn still_url(&self) -> Option<String> {
        self.sample
            .url
            .clone()
            .or_else(|| if self.is_video() { None } else { self.file.url.clone() })
            .or_else(|| self.preview.url.clone())
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Score {
    pub up: i32,
    pub down: i32,
    pub total: i32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FileInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: Option<u64>,
    pub ext: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PreviewInfo {
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct SampleInfo {
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Tags {
    pub general: Vec<String>,
    pub species: Vec<String>,
    pub character: Vec<String>,
    pub copyright: Vec<String>,
    pub artist: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PostsResponse {
    pub posts: Vec<Post>,
}

pub fn search_posts(tags: &str, page: u32, sort: &str, rating: &str) -> Result<Vec<Post>, String> {
    let mut tag_str = tags.to_string();

    if !rating.is_empty() && rating != "all" {
        tag_str.push_str(&format!(" rating:{rating}"));
    }

    if !sort.is_empty() && sort != "default" {
        let order = match sort {
            "score" => "order:score",
            "favcount" => "order:favcount",
            "new" => "order:id_desc",
            "old" => "order:id_asc",
            _ => "",
        };
        if !order.is_empty() {
            tag_str.push_str(&format!(" {order}"));
        }
    }

    let resp = client()?
        .get(format!("{BASE_URL}/posts.json"))
        .query(&[
            ("tags", tag_str.trim()),
            ("page", &page.to_string()),
            ("limit", "50"),
        ])
        .send()
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let data: PostsResponse = resp.json().map_err(|e| e.to_string())?;
    Ok(data.posts)
}

pub fn download_post(post: &Post) -> Result<String, String> {
    let url = post.file.url.as_deref().ok_or("No file URL")?;
    let ext = post.file.ext.as_deref().unwrap_or("bin");

    let dl_dir = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .ok_or("Cannot find downloads directory")?;

    let filename = format!("e621_{}.{}", post.id, ext);
    let path = dl_dir.join(&filename);

    let bytes = get_bytes(url)?;
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}
