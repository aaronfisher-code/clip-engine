import { readFileSync, writeFileSync } from "node:fs";

const CARGO_TOML = "Cargo.toml";
const CARGO_LOCK = "Cargo.lock";
const NPM_MANIFESTS = [
  "package.json",
  "cloud/package.json",
  "packages/contracts/package.json",
];

function parseVersion(value) {
  const match = String(value ?? "")
    .trim()
    .replace(/^v/i, "")
    .match(/^(\d+)\.(\d+)\.(\d+)$/);
  if (!match) {
    return null;
  }
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    text: `${match[1]}.${match[2]}.${match[3]}`,
  };
}

function compare(left, right) {
  if (left.major !== right.major) {
    return left.major - right.major;
  }
  if (left.minor !== right.minor) {
    return left.minor - right.minor;
  }
  return left.patch - right.patch;
}

function bumpVersion(current, bump) {
  if (bump === "major") {
    return `${current.major + 1}.0.0`;
  }
  if (bump === "minor") {
    return `${current.major}.${current.minor + 1}.0`;
  }
  return `${current.major}.${current.minor}.${current.patch + 1}`;
}

function cargoWorkspaceVersion() {
  const text = readFileSync(CARGO_TOML, "utf8");
  const match = text.match(
    /\[workspace\.package\][\s\S]*?^version = "(\d+\.\d+\.\d+)"/m,
  );
  if (!match) {
    throw new Error("Could not read [workspace.package] version from Cargo.toml");
  }
  return match[1];
}

function replaceLockPackageVersion(text, name, version) {
  const pattern = new RegExp(
    `(\\[\\[package\\]\\]\\nname = "${name}"\\nversion = ")[^"]+(")`,
  );
  if (!pattern.test(text)) {
    throw new Error(`Could not update ${name} in Cargo.lock`);
  }
  return text.replace(pattern, `$1${version}$2`);
}

function writeJson(path, mutate) {
  const parsed = JSON.parse(readFileSync(path, "utf8"));
  mutate(parsed);
  writeFileSync(path, `${JSON.stringify(parsed, null, 2)}\n`);
}

export function nextDesktopVersion({ bump = "patch", latestTag } = {}) {
  const versions = [parseVersion(cargoWorkspaceVersion())];
  const tagged = parseVersion(latestTag);
  if (tagged) {
    versions.push(tagged);
  }
  const current = versions
    .filter(Boolean)
    .sort(compare)
    .at(-1);
  if (!current) {
    throw new Error("No current desktop version was found");
  }
  return bumpVersion(current, bump);
}

export function setDesktopVersion(version) {
  const parsed = parseVersion(version);
  if (!parsed) {
    throw new Error(`Invalid version: ${version}`);
  }
  const next = parsed.text;
  const cargo = readFileSync(CARGO_TOML, "utf8");
  const updatedCargo = cargo.replace(
    /(\[workspace\.package\][\s\S]*?^version = ")(\d+\.\d+\.\d+)(")/m,
    `$1${next}$3`,
  );
  if (updatedCargo === cargo) {
    throw new Error("Could not update Cargo.toml version");
  }
  writeFileSync(CARGO_TOML, updatedCargo);

  let lock = readFileSync(CARGO_LOCK, "utf8");
  lock = replaceLockPackageVersion(lock, "clip-engine", next);
  lock = replaceLockPackageVersion(lock, "clip-engine-core", next);
  writeFileSync(CARGO_LOCK, lock);

  for (const manifest of NPM_MANIFESTS) {
    writeJson(manifest, (json) => {
      json.version = next;
      if (json.dependencies?.["@clip-engine/contracts"]) {
        json.dependencies["@clip-engine/contracts"] = next;
      }
    });
  }

  writeJson("package-lock.json", (json) => {
    json.version = next;
    if (json.packages?.[""]) {
      json.packages[""].version = next;
    }
    if (json.packages?.cloud) {
      json.packages.cloud.version = next;
      if (json.packages.cloud.dependencies?.["@clip-engine/contracts"]) {
        json.packages.cloud.dependencies["@clip-engine/contracts"] = next;
      }
    }
    if (json.packages?.["packages/contracts"]) {
      json.packages["packages/contracts"].version = next;
    }
  });

  return next;
}

function argValue(flag) {
  const index = process.argv.indexOf(flag);
  if (index === -1) {
    return undefined;
  }
  return process.argv[index + 1];
}

const command = process.argv[2];
if (command === "current") {
  process.stdout.write(`${cargoWorkspaceVersion()}\n`);
} else if (command === "next") {
  process.stdout.write(
    `${nextDesktopVersion({
      bump: argValue("--bump") ?? "patch",
      latestTag: argValue("--latest-tag"),
    })}\n`,
  );
} else if (command === "set") {
  const version = process.argv[3];
  if (!version) {
    throw new Error("Usage: node scripts/desktop-version.mjs set <version>");
  }
  process.stdout.write(`${setDesktopVersion(version)}\n`);
} else if (command) {
  throw new Error(`Unknown command: ${command}`);
}
