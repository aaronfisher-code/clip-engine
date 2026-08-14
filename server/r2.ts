import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { S3Client } from "@aws-sdk/client-s3";
import { Upload } from "@aws-sdk/lib-storage";
import { config, r2Configured } from "./config.js";

export async function uploadToR2(
  filePath: string,
  key: string,
  onProgress: (progress: number) => void,
) {
  if (!r2Configured()) {
    throw new Error("R2 is not configured. Fill in the R2 values in .env and restart Clip Engine.");
  }
  const file = await stat(filePath);
  const client = new S3Client({
    region: "auto",
    endpoint: `https://${config.r2.accountId}.r2.cloudflarestorage.com`,
    credentials: {
      accessKeyId: config.r2.accessKeyId,
      secretAccessKey: config.r2.secretAccessKey,
    },
  });
  const upload = new Upload({
    client,
    params: {
      Bucket: config.r2.bucket,
      Key: key,
      Body: createReadStream(filePath),
      ContentType: "video/mp4",
      ContentDisposition: `inline; filename="${key.split("/").at(-1)}"`,
      CacheControl: "public, max-age=31536000, immutable",
    },
    queueSize: 4,
    partSize: 10 * 1024 * 1024,
  });
  upload.on("httpUploadProgress", (event) => {
    onProgress(Math.min(1, Number(event.loaded || 0) / file.size));
  });
  await upload.done();
  return `${config.r2.publicBaseUrl}/${key.split("/").map(encodeURIComponent).join("/")}`;
}
