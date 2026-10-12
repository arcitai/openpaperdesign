// Portable path boundary. Adapted from ADS designs.mjs; MIT, see ../LICENSE.
import { lstatSync, realpathSync } from "node:fs";
import { dirname, isAbsolute, join, parse, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

// Installed skills are siblings under the immutable package, independently of cwd.
export const PACKAGE_ROOT = realpathSync(resolve(dirname(fileURLToPath(import.meta.url)), "../../.."));

export function isWithin(parent, candidate) {
  const scoped = relative(parent, candidate);
  return !scoped || (!isAbsolute(scoped) && scoped !== ".." && !scoped.startsWith(`..${sep}`));
}

export function pathExists(path) {
  try { lstatSync(path); return true; }
  catch (error) { if (error.code === "ENOENT") return false; throw error; }
}

// Resolve an existing ancestor first: macOS /tmp and /var aliases are harmless,
// but task-selected symlinks (including dangling ones) must never hide a target.
export function checkedPath(value) {
  if (typeof value !== "string" || !value || !isAbsolute(value))
    throw new Error("Pass an explicit absolute path.");
  const absolute = resolve(value);
  let cursor = parse(absolute).root;
  for (const part of absolute.slice(cursor.length).split(sep).filter(Boolean)) {
    cursor = join(cursor, part);
    if (!pathExists(cursor)) continue;
    if (lstatSync(cursor).isSymbolicLink() && !["/tmp", "/var"].includes(cursor))
      throw new Error(`Path must not traverse a symlink: ${cursor}`);
  }
  let ancestor = absolute;
  while (!pathExists(ancestor)) ancestor = dirname(ancestor);
  return resolve(realpathSync(ancestor), relative(ancestor, absolute));
}

export function externalPath(value) {
  const path = checkedPath(value);
  if (isWithin(PACKAGE_ROOT, path) || isWithin(path, PACKAGE_ROOT))
    throw new Error("Work data must be outside the installed package and its ancestors.");
  return path;
}

export function selectDesign(value) {
  const path = externalPath(value);
  if (!pathExists(path) || !lstatSync(path).isDirectory())
    throw new Error("Selected design must be an existing regular directory.");
  return path;
}

export function sourceFile(root, name) {
  if (!name || isAbsolute(name)) throw new Error("Select a source-relative file.");
  const file = checkedPath(resolve(root, name));
  if (file === root || !isWithin(root, file)) throw new Error("File escapes selected design.");
  if (!pathExists(file) || !lstatSync(file).isFile()) throw new Error(`Missing regular file: ${name}`);
  return file;
}
