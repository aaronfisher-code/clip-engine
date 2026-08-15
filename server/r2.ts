import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { DeleteObjectsCommand, S3Client } from "@aws-sdk/client-s3";
import { Upload } from "@aws-sdk/lib-storage";
import { config, r2Configured } from "./config.js";

type UploadHeaders = {
  contentType: string;
  contentDisposition?: string;
  cacheControl?: string;
};

function configuredClient() {
  if (!r2Configured()) {
    throw new Error("R2 is not configured. Fill in the R2 values in .env and restart Clip Engine.");
  }
  return new S3Client({
    region: "auto",
    endpoint: `https://${config.r2.accountId}.r2.cloudflarestorage.com`,
    credentials: {
      accessKeyId: config.r2.accessKeyId,
      secretAccessKey: config.r2.secretAccessKey,
    },
  });
}

export function publicR2Url(key: string) {
  return `${config.r2.publicBaseUrl}/${key.split("/").map(encodeURIComponent).join("/")}`;
}

export function publicR2Key(url: string, publicBaseUrl = config.r2.publicBaseUrl) {
  try {
    const base = new URL(`${publicBaseUrl.replace(/\/$/, "")}/`);
    const target = new URL(url);
    if (target.origin !== base.origin || !target.pathname.startsWith(base.pathname)) return undefined;
    const encodedKey = target.pathname.slice(base.pathname.length);
    if (!encodedKey) return undefined;
    return encodedKey.split("/").map(decodeURIComponent).join("/");
  } catch {
    return undefined;
  }
}

export async function deleteR2Objects(keys: string[]) {
  const uniqueKeys = [...new Set(keys.filter(Boolean))];
  if (!uniqueKeys.length) return 0;
  const result = await configuredClient().send(new DeleteObjectsCommand({
    Bucket: config.r2.bucket,
    Delete: { Objects: uniqueKeys.map((Key) => ({ Key })), Quiet: true },
  }));
  if (result.Errors?.length) {
    throw new Error(`R2 could not delete ${result.Errors.map((error) => error.Key || "an object").join(", ")}.`);
  }
  return uniqueKeys.length;
}

export async function uploadFileToR2(
  filePath: string,
  key: string,
  headers: UploadHeaders,
  onProgress: (progress: number) => void = () => undefined,
) {
  const file = await stat(filePath);
  const upload = new Upload({
    client: configuredClient(),
    params: {
      Bucket: config.r2.bucket,
      Key: key,
      Body: createReadStream(filePath),
      ContentType: headers.contentType,
      ContentDisposition: headers.contentDisposition,
      CacheControl: headers.cacheControl || "public, max-age=31536000, immutable",
    },
    queueSize: 4,
    partSize: 10 * 1024 * 1024,
  });
  upload.on("httpUploadProgress", (event) => {
    onProgress(Math.min(1, Number(event.loaded || 0) / file.size));
  });
  await upload.done();
  return publicR2Url(key);
}

export async function uploadTextToR2(content: string, key: string, headers: UploadHeaders) {
  const upload = new Upload({
    client: configuredClient(),
    params: {
      Bucket: config.r2.bucket,
      Key: key,
      Body: Buffer.from(content, "utf8"),
      ContentType: headers.contentType,
      ContentDisposition: headers.contentDisposition,
      CacheControl: headers.cacheControl || "public, max-age=31536000, immutable",
    },
  });
  await upload.done();
  return publicR2Url(key);
}
