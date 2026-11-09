import { copyFile, mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { join, relative } from "node:path";

const [root, repository, tag, releaseDirectory] = process.argv.slice(2);
if (!root || !repository || !tag || !releaseDirectory) {
  throw new Error("Usage: node scripts/create-updater-json.mjs <artifacts> <owner/repo> <tag> <release-directory>");
}

async function walk(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(entries.map(async (entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? walk(path) : [path];
  }));
  return nested.flat();
}

const files = await walk(root);
const signatures = files.filter((file) => file.endsWith(".sig"));
const platforms = {};
const assets = new Map();
const publishedSources = new Map();
await mkdir(releaseDirectory, { recursive: true });

function uniqueAssetName(name, source) {
  if (!assets.has(name)) return name;
  const artifactName = relative(root, source).split(/[\\/]/)[0];
  const dot = name.lastIndexOf(".");
  return dot === -1 ? `${name}-${artifactName}` : `${name.slice(0, dot)}-${artifactName}${name.slice(dot)}`;
}

async function addAsset(source, preferredName) {
  const existing = publishedSources.get(source);
  if (existing) return existing;
  const name = uniqueAssetName(preferredName, source);
  const destination = join(releaseDirectory, name);
  await copyFile(source, destination);
  assets.set(name, destination);
  publishedSources.set(source, name);
  return name;
}

for (const signatureFile of signatures) {
  const artifact = signatureFile.slice(0, -4);
  const name = artifact.split(/[\\/]/).at(-1);
  const path = relative(root, artifact).replaceAll("\\", "/");
  const artifactRoot = path.split("/")[0];
  let target;
  if (name.endsWith("-setup.exe") || name.endsWith(".nsis.zip")) target = "windows-x86_64";
  else if (name.endsWith(".msi") || name.endsWith(".msi.zip")) target = "windows-x86_64";
  else if (name.endsWith(".AppImage.tar.gz")) target = "linux-x86_64";
  else if (name.endsWith(".app.tar.gz")) {
    if (artifactRoot.includes("macos-apple-silicon")) target = "darwin-aarch64";
    if (artifactRoot.includes("macos-intel")) target = "darwin-x86_64";
  }
  if (!target) continue;
  if (target === "windows-x86_64" && platforms[target]?.url.includes("-setup.exe")) continue;
  const updateAssetName = target.startsWith("darwin-")
    ? name.replace(".app.tar.gz", `-${artifactRoot.replace("nomad-", "")}.app.tar.gz`)
    : name;
  const publishedName = await addAsset(artifact, updateAssetName);
  const signatureName = await addAsset(signatureFile, `${publishedName}.sig`);
  platforms[target] = {
    signature: (await readFile(signatureFile, "utf8")).trim(),
    url: `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(publishedName)}`,
  };
}

for (const file of files) {
  if (/\.(msi|exe|deb|rpm|AppImage|dmg)$/i.test(file)) {
    await addAsset(file, file.split(/[\\/]/).at(-1));
  }
}

for (const target of ["windows-x86_64", "linux-x86_64", "darwin-aarch64", "darwin-x86_64"]) {
  if (!platforms[target]) throw new Error(`Missing signed updater artifact for ${target}`);
}

await writeFile(join(releaseDirectory, "latest.json"), `${JSON.stringify({
  version: tag.replace(/^v/, ""),
  notes: `Nomad ${tag}`,
  pub_date: new Date().toISOString(),
  platforms,
}, null, 2)}\n`);
console.log(`Created updater feed for ${Object.keys(platforms).join(", ")}`);
