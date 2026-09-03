/// Validates `name` as an archive file name: a `.md` file made of lowercase
/// alphanumerics and single interior hyphens, nothing else.
///
/// Mirrors `valid_archive_name` in `crates/core/src/archive.rs` message for
/// message, so the name field can reject bad input before the backend ever
/// sees it rather than round-tripping to learn what's wrong.
export function validArchiveName(name: string): string | null {
  if (name.length === 0) {
    return "name is empty";
  }
  if (name.includes("/") || name.includes("\\")) {
    return "name contains a path separator";
  }
  if (name.includes("..")) {
    return 'name contains ".."';
  }
  if (!name.endsWith(".md")) {
    return 'name must end in ".md"';
  }
  const stem = name.slice(0, -".md".length);
  if (stem.length === 0) {
    return "name is empty";
  }
  if (stem.startsWith("-")) {
    return "name starts with a hyphen";
  }
  if (stem.endsWith("-")) {
    return "name ends with a hyphen";
  }
  if (stem.includes("--")) {
    return "name contains consecutive hyphens";
  }
  for (const c of stem) {
    if (!/[a-z0-9-]/.test(c)) {
      return `name contains an invalid character: '${c}'`;
    }
  }
  return null;
}
