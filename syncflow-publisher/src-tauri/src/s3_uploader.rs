//! Uploads one join's recording folder to S3, one object per file.
//!
//! Files upload in parallel, each reporting its own progress. `manifest.json` goes last
//! and only once every other file is up, so its presence in S3 means the join is
//! complete. Progress is kept in `upload-state.json` in the folder, so a retry sends
//! only what is missing.

use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use livekit_gstreamer::composite::{
    naming::{metadata_file_name, MANIFEST_FILE_NAME},
    RecordingManifest,
};
use rusoto_core::RusotoError;
use rusoto_s3::{
    AbortMultipartUploadRequest, CompleteMultipartUploadRequest, CompletedMultipartUpload,
    CompletedPart, CreateMultipartUploadRequest, HeadObjectRequest, PutObjectRequest, S3Client,
    UploadPartRequest, S3,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::AsyncReadExt,
    sync::{mpsc::Sender, Semaphore},
    task::JoinSet,
};

pub const UPLOAD_STATE_FILE: &str = "upload-state.json";

/// Files uploaded at the same time.
const PARALLEL_FILES: usize = 3;
/// Attempts per S3 request before a file is marked failed.
const ATTEMPTS: u32 = 4;
const MIB: u64 = 1024 * 1024;
const MIN_PART_SIZE: u64 = 8 * MIB;
/// S3 allows 10,000 parts; staying below leaves headroom for a file that grows.
const MAX_PARTS: u64 = 9_000;

#[derive(Debug, Error)]
pub enum S3UploaderError {
    #[error("Failed to read the recording: {0}")]
    FileRead(#[from] std::io::Error),

    #[error("Failed to read the recording's manifest: {0}")]
    Manifest(#[from] serde_json::Error),

    #[error("s3://{0} already holds a recording; not overwriting it")]
    AlreadyUploaded(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileStatus {
    Pending,
    Uploading,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileState {
    pub size: u64,
    pub status: FileStatus,
    pub error: Option<String>,
}

/// Where a join uploads to and how far it got. Saved as `upload-state.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadState {
    pub session_id: String,
    pub session_name: String,
    pub join: u32,
    pub bucket: String,
    /// Key prefix without a trailing slash; files go to `<prefix>/<file>`.
    pub prefix: String,
    pub files: BTreeMap<String, FileState>,
}

impl UploadState {
    pub fn new(
        session_id: &str,
        session_name: &str,
        join: u32,
        bucket: &str,
        prefix: &str,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            session_name: session_name.into(),
            join,
            bucket: bucket.into(),
            prefix: prefix.trim_end_matches('/').into(),
            files: BTreeMap::new(),
        }
    }

    pub fn load(dir: &Path) -> Option<Self> {
        let json = std::fs::read(dir.join(UPLOAD_STATE_FILE)).ok()?;
        serde_json::from_slice(&json).ok()
    }

    /// Writes to a temporary file and renames it, so a crash never leaves a torn state.
    fn save(&self, dir: &Path) -> std::io::Result<()> {
        let tmp = dir.join(format!("{UPLOAD_STATE_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, dir.join(UPLOAD_STATE_FILE))
    }

    /// True when every file, including the manifest, is in S3.
    pub fn is_complete(&self) -> bool {
        !self.files.is_empty() && self.files.values().all(|f| f.status == FileStatus::Done)
    }

    fn key(&self, file: &str) -> String {
        format!("{}/{file}", self.prefix)
    }

    fn set(&mut self, file: &str, size: u64, result: Result<(), String>) {
        let (status, error) = match result {
            Ok(()) => (FileStatus::Done, None),
            Err(e) => (FileStatus::Failed, Some(e)),
        };
        self.files.insert(
            file.into(),
            FileState {
                size,
                status,
                error,
            },
        );
    }
}

/// Every join folder under `recordings_dir` (`<session>/<join>/`) that has an
/// `upload-state.json`, with that state, sorted by folder path.
pub fn saved_uploads(recordings_dir: &Path) -> Vec<(PathBuf, UploadState)> {
    let subdirs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| Some(e.ok()?.path()))
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut uploads: Vec<_> = subdirs(recordings_dir)
        .iter()
        .flat_map(|session| subdirs(session))
        .filter_map(|join| UploadState::load(&join).map(|state| (join, state)))
        .collect();
    uploads.sort_by(|a, b| a.0.cmp(&b.0));
    uploads
}

/// One file's progress, sent on every status change and after each part.
#[derive(Debug, Clone)]
pub struct UploadEvent {
    pub file: String,
    pub uploaded_bytes: u64,
    pub total_bytes: u64,
    pub status: FileStatus,
    pub error: Option<String>,
}

async fn send(events: &Option<Sender<UploadEvent>>, event: UploadEvent) {
    if let Some(tx) = events {
        let _ = tx.send(event).await;
    }
}

/// Uploads the join folder `dir` as described by `state`, skipping files a previous run
/// already uploaded. Returns the final state; check [`UploadState::is_complete`].
///
/// On a first upload (no `upload-state.json` yet) it refuses to write over a
/// `manifest.json` already at the prefix, which happens if the local folder was deleted
/// and the join number reused.
pub async fn upload_join(
    dir: &Path,
    mut state: UploadState,
    s3_client: &S3Client,
    events: Option<Sender<UploadEvent>>,
) -> Result<UploadState, S3UploaderError> {
    if let Some(saved) = UploadState::load(dir) {
        if saved.bucket == state.bucket && saved.prefix == state.prefix {
            state.files = saved.files;
        }
    } else {
        let manifest_key = state.key(MANIFEST_FILE_NAME);
        if object_exists(s3_client, &state.bucket, &manifest_key).await {
            return Err(S3UploaderError::AlreadyUploaded(format!(
                "{}/{}",
                state.bucket, state.prefix
            )));
        }
    }

    let files = upload_files(dir)?;
    let mut sizes = BTreeMap::new();
    for file in &files {
        sizes.insert(
            file.clone(),
            tokio::fs::metadata(dir.join(file)).await?.len(),
        );
    }
    // Forget files that are no longer in the folder's manifest.
    state.files.retain(|name, _| sizes.contains_key(name));

    let is_done = |state: &UploadState, file: &str| {
        state
            .files
            .get(file)
            .is_some_and(|f| f.status == FileStatus::Done && f.size == sizes[file])
    };
    let mut todo = Vec::new();
    for file in &files {
        let size = sizes[file];
        if is_done(&state, file) {
            send(&events, event(file, size, size, FileStatus::Done, None)).await;
        } else {
            state.files.insert(
                file.clone(),
                FileState {
                    size,
                    status: FileStatus::Pending,
                    error: None,
                },
            );
            send(&events, event(file, 0, size, FileStatus::Pending, None)).await;
            todo.push(file.clone());
        }
    }
    state.save(dir)?;

    let (manifest, media): (Vec<_>, Vec<_>) =
        todo.into_iter().partition(|f| f == MANIFEST_FILE_NAME);

    let limit = Arc::new(Semaphore::new(PARALLEL_FILES));
    let mut tasks = JoinSet::new();
    for file in media {
        let job = FileJob {
            client: s3_client.clone(),
            bucket: state.bucket.clone(),
            key: state.key(&file),
            path: dir.join(&file),
            size: sizes[&file],
            file,
            events: events.clone(),
        };
        let limit = limit.clone();
        tasks.spawn(async move {
            let _permit = limit.acquire_owned().await;
            let result = job.run().await;
            (job.file, job.size, result)
        });
    }
    while let Some(joined) = tasks.join_next().await {
        // A panicking task loses its file name; the file stays `Pending` and is retried.
        let Ok((file, size, result)) = joined else {
            continue;
        };
        state.set(&file, size, result);
        state.save(dir)?;
    }

    let media_done = files
        .iter()
        .filter(|f| *f != MANIFEST_FILE_NAME)
        .all(|f| is_done(&state, f));
    if media_done {
        for file in manifest {
            let job = FileJob {
                client: s3_client.clone(),
                bucket: state.bucket.clone(),
                key: state.key(&file),
                path: dir.join(&file),
                size: sizes[&file],
                file,
                events: events.clone(),
            };
            let result = job.run().await;
            state.set(&job.file, job.size, result);
            state.save(dir)?;
        }
    }
    Ok(state)
}

fn event(
    file: &str,
    uploaded_bytes: u64,
    total_bytes: u64,
    status: FileStatus,
    error: Option<String>,
) -> UploadEvent {
    UploadEvent {
        file: file.into(),
        uploaded_bytes,
        total_bytes,
        status,
        error,
    }
}

/// The files to upload, from the folder's manifest: each output's file and its metadata
/// file when they exist, then `manifest.json` last. Leftovers such as `*.tmp` and
/// `upload-state.json` are not listed, so they are never uploaded.
pub fn upload_files(dir: &Path) -> Result<Vec<String>, S3UploaderError> {
    let manifest: RecordingManifest =
        serde_json::from_slice(&std::fs::read(dir.join(MANIFEST_FILE_NAME))?)?;
    let mut files = Vec::new();
    for output in &manifest.outputs {
        for file in [output.file.clone(), metadata_file_name(&output.file)] {
            if dir.join(&file).is_file() && !files.contains(&file) {
                files.push(file);
            }
        }
    }
    files.push(MANIFEST_FILE_NAME.to_string());
    Ok(files)
}

fn content_type(file: &str) -> &'static str {
    match file.rsplit_once('.').map(|(_, ext)| ext) {
        Some("mp4") => "video/mp4",
        Some("m4a") => "audio/mp4",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// Part size for a file of `size` bytes: at least 8 MiB, a whole number of MiB, and
/// large enough to stay under [`MAX_PARTS`] parts.
pub fn part_size(size: u64) -> u64 {
    (size.div_ceil(MAX_PARTS).div_ceil(MIB) * MIB).max(MIN_PART_SIZE)
}

async fn object_exists(client: &S3Client, bucket: &str, key: &str) -> bool {
    let request = HeadObjectRequest {
        bucket: bucket.into(),
        key: key.into(),
        ..Default::default()
    };
    match client.head_object(request).await {
        Ok(_) => true,
        Err(RusotoError::Service(_)) => false,
        Err(RusotoError::Unknown(r)) if r.status.as_u16() == 404 => false,
        // No permission to check, or offline: upload as usual rather than not at all.
        Err(e) => {
            eprintln!("Could not check s3://{bucket}/{key} before uploading: {e}");
            false
        }
    }
}

/// Retries network failures, throttling and server errors; not bad requests or
/// denied access, which won't succeed on a second try.
fn is_retryable<E>(error: &RusotoError<E>) -> bool {
    match error {
        RusotoError::HttpDispatch(_) => true,
        RusotoError::Unknown(r) => {
            let status = r.status.as_u16();
            r.status.is_server_error() || status == 408 || status == 429
        }
        _ => false,
    }
}

async fn with_retry<T, E, F, Fut>(what: &str, mut request: F) -> Result<T, String>
where
    E: std::error::Error + 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, RusotoError<E>>>,
{
    let mut attempt = 0;
    loop {
        match request().await {
            Ok(value) => return Ok(value),
            Err(e) if attempt + 1 < ATTEMPTS && is_retryable(&e) => {
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                attempt += 1;
            }
            Err(e) => return Err(format!("{what} failed: {e}")),
        }
    }
}

struct FileJob {
    client: S3Client,
    bucket: String,
    key: String,
    path: PathBuf,
    file: String,
    size: u64,
    events: Option<Sender<UploadEvent>>,
}

impl FileJob {
    async fn run(&self) -> Result<(), String> {
        self.progress(0, FileStatus::Uploading, None).await;
        let result = if self.size <= part_size(self.size) {
            self.put().await
        } else {
            self.multipart().await
        };
        match &result {
            Ok(()) => self.progress(self.size, FileStatus::Done, None).await,
            Err(e) => self.progress(0, FileStatus::Failed, Some(e.clone())).await,
        }
        result
    }

    async fn progress(&self, uploaded: u64, status: FileStatus, error: Option<String>) {
        let update = event(&self.file, uploaded, self.size, status, error);
        send(&self.events, update).await;
    }

    /// One request for small files, including empty ones (a multipart needs a part).
    async fn put(&self) -> Result<(), String> {
        let body = tokio::fs::read(&self.path)
            .await
            .map_err(|e| format!("Reading {} failed: {e}", self.file))?;
        with_retry("Upload", || {
            self.client.put_object(PutObjectRequest {
                bucket: self.bucket.clone(),
                key: self.key.clone(),
                content_type: Some(content_type(&self.file).into()),
                content_length: Some(body.len() as i64),
                body: Some(body.clone().into()),
                ..Default::default()
            })
        })
        .await?;
        Ok(())
    }

    async fn multipart(&self) -> Result<(), String> {
        let created = with_retry("Starting the upload", || {
            self.client
                .create_multipart_upload(CreateMultipartUploadRequest {
                    bucket: self.bucket.clone(),
                    key: self.key.clone(),
                    content_type: Some(content_type(&self.file).into()),
                    ..Default::default()
                })
        })
        .await?;
        let upload_id = created
            .upload_id
            .ok_or_else(|| "S3 returned no upload id".to_string())?;

        let result = self.upload_parts(&upload_id).await;
        if result.is_err() {
            // Otherwise the uploaded parts stay in the bucket (and are billed).
            let _ = self
                .client
                .abort_multipart_upload(AbortMultipartUploadRequest {
                    bucket: self.bucket.clone(),
                    key: self.key.clone(),
                    upload_id: upload_id.clone(),
                    ..Default::default()
                })
                .await;
        }
        result
    }

    async fn upload_parts(&self, upload_id: &str) -> Result<(), String> {
        let read_error = |e: std::io::Error| format!("Reading {} failed: {e}", self.file);
        let mut file = tokio::fs::File::open(&self.path)
            .await
            .map_err(read_error)?;
        let part_size = part_size(self.size) as usize;
        let mut buf = vec![0u8; part_size];
        let mut parts = Vec::new();
        let mut uploaded = 0u64;

        loop {
            let mut filled = 0;
            while filled < part_size {
                let n = file.read(&mut buf[filled..]).await.map_err(read_error)?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            if filled == 0 {
                break;
            }
            let part_number = parts.len() as i64 + 1;
            let part = &buf[..filled];
            let response = with_retry("Uploading a part", || {
                self.client.upload_part(UploadPartRequest {
                    bucket: self.bucket.clone(),
                    key: self.key.clone(),
                    upload_id: upload_id.into(),
                    part_number,
                    content_length: Some(filled as i64),
                    body: Some(part.to_vec().into()),
                    ..Default::default()
                })
            })
            .await?;
            parts.push(CompletedPart {
                e_tag: response.e_tag,
                part_number: Some(part_number),
            });
            uploaded += filled as u64;
            self.progress(uploaded, FileStatus::Uploading, None).await;
            if filled < part_size {
                break;
            }
        }

        with_retry("Finishing the upload", || {
            self.client
                .complete_multipart_upload(CompleteMultipartUploadRequest {
                    bucket: self.bucket.clone(),
                    key: self.key.clone(),
                    upload_id: upload_id.into(),
                    multipart_upload: Some(CompletedMultipartUpload {
                        parts: Some(parts.clone()),
                    }),
                    ..Default::default()
                })
        })
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_sizes() {
        assert_eq!(part_size(0), MIN_PART_SIZE);
        assert_eq!(part_size(MIN_PART_SIZE), MIN_PART_SIZE);
        assert_eq!(part_size(MIN_PART_SIZE + 1), MIN_PART_SIZE);
        // 200 GiB needs bigger parts to stay under the part limit.
        let huge = 200 * 1024 * MIB;
        let size = part_size(huge);
        assert_eq!(size % MIB, 0);
        assert!(huge.div_ceil(size) <= MAX_PARTS);
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type("av-mix.mp4"), "video/mp4");
        assert_eq!(content_type("mic-1_USB.m4a"), "audio/mp4");
        assert_eq!(content_type("manifest.json"), "application/json");
        assert_eq!(content_type("notes"), "application/octet-stream");
    }

    #[test]
    fn files_come_from_the_manifest_with_the_manifest_last() {
        let dir = std::env::temp_dir().join(format!("upload-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = serde_json::json!({
            "status": "completed",
            "clock": "monotonic-system",
            "baseTimeNs": null,
            "wallclockAtBaseNs": null,
            "devices": [],
            "roles": [],
            "outputs": [
                { "id": "av-mix", "file": "av-mix.mp4", "roles": [], "audioChannels": null,
                  "status": "finalized", "firstPtsNs": null, "lastPtsNs": null, "error": null },
                { "id": "mic-1", "file": "mic-1_USB.m4a", "roles": [], "audioChannels": null,
                  "status": "failed", "firstPtsNs": null, "lastPtsNs": null, "error": null }
            ],
            "audioContinuity": [],
            "events": [],
            "context": null
        });
        std::fs::write(dir.join(MANIFEST_FILE_NAME), manifest.to_string()).unwrap();
        for file in [
            "av-mix.mp4",
            "av-mix.json",
            "manifest.json.tmp",
            UPLOAD_STATE_FILE,
        ] {
            std::fs::write(dir.join(file), "x").unwrap();
        }
        // mic-1 failed before writing anything, so it has no file.
        assert_eq!(
            upload_files(&dir).unwrap(),
            ["av-mix.mp4", "av-mix.json", MANIFEST_FILE_NAME]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn state_round_trips_and_reports_completion() {
        let dir = std::env::temp_dir().join(format!("upload-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = UploadState::new("id", "Lecture", 2, "bucket", "p/q/");
        assert_eq!(state.key("a.mp4"), "p/q/a.mp4");
        assert!(!state.is_complete());
        state.set("a.mp4", 3, Ok(()));
        state.set(MANIFEST_FILE_NAME, 1, Err("down".into()));
        state.save(&dir).unwrap();
        let loaded = UploadState::load(&dir).unwrap();
        assert_eq!(loaded.join, 2);
        assert!(!loaded.is_complete());
        assert_eq!(
            loaded.files[MANIFEST_FILE_NAME].error.as_deref(),
            Some("down")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Uploads against a local S3 server, e.g. `moto_server -p 19000`:
    /// `S3_TEST_ENDPOINT=http://127.0.0.1:19000 cargo test -p syncflow-publisher -- --ignored`
    #[tokio::test]
    #[ignore = "needs a local S3 server in S3_TEST_ENDPOINT"]
    async fn uploads_retries_and_refuses_to_overwrite() {
        use rusoto_s3::{CreateBucketRequest, ListObjectsV2Request};

        let endpoint = std::env::var("S3_TEST_ENDPOINT").expect("S3_TEST_ENDPOINT");
        let client = S3Client::new_with(
            rusoto_core::HttpClient::new().unwrap(),
            rusoto_credential::StaticProvider::new_minimal("test".into(), "test".into()),
            rusoto_core::Region::Custom {
                name: "us-east-1".into(),
                endpoint,
            },
        );
        let bucket = format!("upload-test-{}", std::process::id());
        let dir = std::env::temp_dir().join(format!("upload-int-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let output = |id: &str, file: &str| {
            serde_json::json!({ "id": id, "file": file, "roles": [], "audioChannels": null,
                "status": "finalized", "firstPtsNs": null, "lastPtsNs": null, "error": null })
        };
        let manifest = serde_json::json!({
            "status": "completed", "clock": "monotonic-system", "baseTimeNs": null,
            "wallclockAtBaseNs": null, "devices": [], "roles": [],
            "outputs": [output("av-mix", "av-mix.mp4"), output("mic-1", "mic-1_USB.m4a")],
            "audioContinuity": [], "events": [], "context": null
        });
        std::fs::write(dir.join(MANIFEST_FILE_NAME), manifest.to_string()).unwrap();
        // Three parts of 8 MiB, 8 MiB and 4 MiB + 1 byte; an empty file; a small one.
        let big = 20 * MIB + 1;
        std::fs::write(dir.join("av-mix.mp4"), vec![7u8; big as usize]).unwrap();
        std::fs::write(dir.join("av-mix.json"), "{}").unwrap();
        std::fs::write(dir.join("mic-1_USB.m4a"), "").unwrap();
        let prefix = "proj-1/Lecture 3/local-recordings/dev-abc(g)/1";
        let fresh = || UploadState::new("sid", "Lecture 3", 1, &bucket, prefix);

        // No bucket yet: every file fails, the manifest is held back.
        let state = upload_join(&dir, fresh(), &client, None).await.unwrap();
        assert!(!state.is_complete());
        assert_eq!(state.files["av-mix.mp4"].status, FileStatus::Failed);
        assert_eq!(state.files[MANIFEST_FILE_NAME].status, FileStatus::Pending);

        // Retry once the bucket exists: everything goes up, the manifest last.
        client
            .create_bucket(CreateBucketRequest {
                bucket: bucket.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1000);
        let state = upload_join(&dir, fresh(), &client, Some(tx)).await.unwrap();
        assert!(state.is_complete(), "{state:?}");
        let mut done = Vec::new();
        let mut parts = 0;
        while let Some(e) = rx.recv().await {
            if e.status == FileStatus::Done {
                done.push(e.file.clone());
            }
            if e.file == "av-mix.mp4" && e.status == FileStatus::Uploading && e.uploaded_bytes > 0 {
                parts += 1;
            }
        }
        assert_eq!(done.last().map(String::as_str), Some(MANIFEST_FILE_NAME));
        assert_eq!(parts, 3);

        let listed = client
            .list_objects_v2(ListObjectsV2Request {
                bucket: bucket.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut keys: Vec<(String, i64)> = listed
            .contents
            .unwrap_or_default()
            .into_iter()
            .map(|o| (o.key.unwrap(), o.size.unwrap()))
            .collect();
        keys.sort();
        let key = |f: &str| format!("{prefix}/{f}");
        assert_eq!(
            keys,
            [
                (key("av-mix.json"), 2),
                (key("av-mix.mp4"), big as i64),
                (key(MANIFEST_FILE_NAME), manifest.to_string().len() as i64),
                (key("mic-1_USB.m4a"), 0),
            ]
        );
        let head = client
            .head_object(HeadObjectRequest {
                bucket: bucket.clone(),
                key: key("av-mix.mp4"),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(head.content_type.as_deref(), Some("video/mp4"));

        // Running again uploads nothing.
        let (tx, mut rx) = tokio::sync::mpsc::channel(1000);
        upload_join(&dir, fresh(), &client, Some(tx)).await.unwrap();
        while let Some(e) = rx.recv().await {
            assert_eq!(e.status, FileStatus::Done, "{} was uploaded again", e.file);
        }

        // Without local state (folder deleted, join number reused), it won't overwrite.
        std::fs::remove_file(dir.join(UPLOAD_STATE_FILE)).unwrap();
        let err = upload_join(&dir, fresh(), &client, None).await.unwrap_err();
        assert!(matches!(err, S3UploaderError::AlreadyUploaded(_)), "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
