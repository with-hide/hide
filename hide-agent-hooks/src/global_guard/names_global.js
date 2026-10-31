/** Whether `text` can name the Hide global instruction, or the file a link at it points to, which is the one every
 * session is given; names are matched as the helper's guard matches them. */
function namesGlobal(text) {
  if (typeof text !== "string") return false;
  const lower = text.toLowerCase();
  if (lower.includes(GLOBAL_INSTRUCTION)) return true;
  return STATE_DIRS.some((dir) => {
    let target;
    try {
      target = readlinkSync(path.join(dir, GLOBAL_INSTRUCTION));
    } catch {
      return false;
    }
    const name = path.basename(target).toLowerCase();
    return name !== "" && lower.includes(name);
  });
}
