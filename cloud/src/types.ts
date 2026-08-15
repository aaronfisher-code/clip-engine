export interface Env {
  DB: D1Database;
  MEDIA: R2Bucket;
  APP_BASE_URL: string;
  MEDIA_BASE_URL: string;
  R2_ACCOUNT_ID: string;
  R2_BUCKET_NAME: string;
  R2_PARENT_ACCESS_KEY_ID: string;
  CLIP_TTL_DAYS: string;
  MAX_UPLOAD_BYTES: string;
  BOOTSTRAP_TOKEN: string;
  TOKEN_PEPPER: string;
  R2_PARENT_SECRET_ACCESS_KEY: string;
}

export type Principal = {
  userId: string;
  deviceId: string;
  username: string;
  displayName: string;
  role: "owner" | "member";
};
