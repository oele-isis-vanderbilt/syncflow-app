use rusoto_s3::{
    CreateMultipartUploadError, CreateMultipartUploadRequest, S3Client, UploadPartRequest, S3,
};
use std::{fs::File, path::PathBuf};
use tokio::sync::mpsc::Sender;

use zip::ZipWriter;
use zip_extensions::write::ZipWriterExtensions;

use tokio::fs::File as TokioFile;
use tokio::io::AsyncReadExt;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum S3UploaderError {
    #[error("Failed to read file: {0}")]
    FileReadError(#[from] std::io::Error),

    #[error("Failed to create multipart upload: {0}")]
    CreateMultipartUploadError(#[from] rusoto_core::RusotoError<CreateMultipartUploadError>),

    #[error("Failed to upload part: {0}")]
    UploadPartError(#[from] rusoto_core::RusotoError<rusoto_s3::UploadPartError>),

    #[error("Failed to complete multipart upload: {0}")]
    CompleteMultipartUploadError(
        #[from] rusoto_core::RusotoError<rusoto_s3::CompleteMultipartUploadError>,
    ),
}

fn archive_directory(directory: &PathBuf, zip_path: &PathBuf) -> Result<PathBuf, std::io::Error> {
    let file = File::create(zip_path)?;
    let zip = ZipWriter::new(file);
    zip.create_from_directory(&directory)?;
    Ok(zip_path.clone())
}

pub async fn upload_to_s3(
    directory: &PathBuf,
    bucket: &str,
    key: &str,
    s3_client: &S3Client,
    mut progress_emitter: Option<Sender<f32>>,
) -> Result<(), S3UploaderError> {
    let zip_path = directory.with_extension("zip");
    archive_directory(directory, &zip_path)?;

    let mut file = TokioFile::open(&zip_path).await?;
    let key_with_extension = format!("{}.zip", key);

    let multipart = s3_client
        .create_multipart_upload(CreateMultipartUploadRequest {
            bucket: bucket.to_string(),
            key: key_with_extension.clone(),
            content_type: Some("application/zip".to_string()),
            ..Default::default()
        })
        .await?;

    let upload_id = multipart
        .upload_id
        .clone()
        .expect("S3 must return upload_id");

    let part_size: usize = 5 * 1024 * 1024;
    let mut buf = vec![0u8; part_size];
    let mut filled = 0usize;
    let mut part_number: i64 = 1;
    let mut etags: Vec<String> = Vec::new();

    let total_len = tokio::fs::metadata(&zip_path).await?.len();
    let mut uploaded: u64 = 0;

    loop {
        let n = file.read(&mut buf[filled..]).await?;
        if n == 0 {
            if filled > 0 {
                let part_bytes: Vec<u8> = buf[..filled].to_vec();
                let resp = s3_client
                    .upload_part(rusoto_s3::UploadPartRequest {
                        bucket: bucket.to_string(),
                        key: key_with_extension.clone(),
                        upload_id: upload_id.clone(),
                        part_number,
                        content_length: Some(filled as i64),
                        body: Some(part_bytes.into()),
                        ..Default::default()
                    })
                    .await?;
                etags.push(resp.e_tag.unwrap());
                uploaded += filled as u64;

                if let Some(tx) = &mut progress_emitter {
                    let _ = tx.send((uploaded as f32 / total_len as f32) * 100.0).await;
                }
            }
            break;
        }

        filled += n;

        if filled == part_size {
            let part_bytes: Vec<u8> = buf[..filled].to_vec();
            let resp = s3_client
                .upload_part(rusoto_s3::UploadPartRequest {
                    bucket: bucket.to_string(),
                    key: key_with_extension.clone(),
                    upload_id: upload_id.clone(),
                    part_number,
                    content_length: Some(filled as i64),
                    body: Some(part_bytes.into()),
                    ..Default::default()
                })
                .await?;
            etags.push(resp.e_tag.unwrap());
            uploaded += filled as u64;

            if let Some(tx) = &mut progress_emitter {
                let _ = tx.send((uploaded as f32 / total_len as f32) * 100.0).await;
            }

            part_number += 1;
            filled = 0;
        }
    }

    let completed_parts: Vec<rusoto_s3::CompletedPart> = etags
        .into_iter()
        .enumerate()
        .map(|(i, e_tag)| rusoto_s3::CompletedPart {
            e_tag: Some(e_tag),
            part_number: Some((i + 1) as i64),
        })
        .collect();

    let completed_upload = rusoto_s3::CompletedMultipartUpload {
        parts: Some(completed_parts),
    };

    s3_client
        .complete_multipart_upload(rusoto_s3::CompleteMultipartUploadRequest {
            bucket: bucket.to_string(),
            key: key_with_extension,
            upload_id,
            multipart_upload: Some(completed_upload),
            ..Default::default()
        })
        .await?;

    Ok(())
}
