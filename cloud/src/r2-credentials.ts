const encoder = new TextEncoder();

function base64(bytes: Uint8Array) {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function base64Url(bytes: Uint8Array) {
  return base64(bytes).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

function jsonPart(value: unknown) {
  return base64Url(encoder.encode(JSON.stringify(value)));
}

export async function createR2TemporaryCredentials(input: {
  accountId: string;
  accessKeyId: string;
  secretAccessKey: string;
  bucket: string;
  objects: string[];
  ttlSeconds: number;
  nowSeconds?: number;
}) {
  const endpoint = `https://${input.accountId}.r2.cloudflarestorage.com`;
  const issuedAt = input.nowSeconds ?? Math.floor(Date.now() / 1_000);
  const header = jsonPart({ alg: "HS256", typ: "JWT" });
  const claims = jsonPart({
    bucket: input.bucket,
    scope: "object-read-write",
    paths: { prefixPaths: [], objectPaths: input.objects },
    sub: input.accountId,
    iss: input.accessKeyId,
    aud: new URL(endpoint).host,
    iat: issuedAt,
    exp: issuedAt + input.ttlSeconds,
  });
  const unsignedToken = `${header}.${claims}`;
  const signingKey = await crypto.subtle.importKey(
    "raw",
    encoder.encode(input.secretAccessKey),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signature = new Uint8Array(await crypto.subtle.sign("HMAC", signingKey, encoder.encode(unsignedToken)));
  const token = `${unsignedToken}.${base64Url(signature)}`;
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", encoder.encode(token)));
  const secretAccessKey = [...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("");

  return {
    accessKeyId: input.accessKeyId,
    secretAccessKey,
    sessionToken: base64(encoder.encode(`jwt/${token}`)),
    endpoint,
    bucket: input.bucket,
    expiresAt: new Date((issuedAt + input.ttlSeconds) * 1_000).toISOString(),
  };
}
