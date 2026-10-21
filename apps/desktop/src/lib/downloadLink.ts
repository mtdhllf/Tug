const PAGE_EXTENSIONS = new Set([
  "html",
  "htm",
  "php",
  "asp",
  "aspx",
  "jsp",
  "do",
  "action",
  "xhtml",
]);

/** 去掉剪贴板文本的空白和成对引号。 */
export function normalizeClipboardText(value: string): string {
  return value.trim().replace(/^["']+|["']+$/g, "").trim();
}

/** Windows 本地 .torrent 路径（含 UNC）。 */
export function isTorrentPath(value: string): boolean {
  return /^(?:[a-zA-Z]:[\\/]|\\\\)[^\r\n]*\.torrent$/i.test(value.trim());
}

/** 磁力链接，至少要有 btih。 */
export function isMagnetLink(value: string): boolean {
  return /^magnet:\?/i.test(value) && /[?&]xt=urn:btih:/i.test(value);
}

/**
 * HTTP(S) 链接且路径最后一段带文件扩展名。
 * 纯网址（例如 https://example.com/ 或 /docs）不触发剪贴板提醒。
 */
export function isHttpFileLink(value: string): boolean {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return false;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return false;

  const segment = url.pathname.split("/").filter(Boolean).pop();
  if (!segment) return false;
  const match = /\.([a-zA-Z0-9]{2,8})$/.exec(segment);
  if (!match) return false;
  return !PAGE_EXTENSIONS.has(match[1].toLowerCase());
}

/** 是否是值得提示的下载候选：HTTP 文件、磁力、本地 .torrent 路径。 */
export function isDownloadCandidate(value: string): boolean {
  const text = normalizeClipboardText(value);
  if (!text) return false;
  return isMagnetLink(text) || isTorrentPath(text) || isHttpFileLink(text);
}

/**
 * 从多行剪贴板文本中提取第一个下载候选；没有则返回 null。
 * 这样可以忽略从聊天/网页复制时附带的多余文字。
 */
export function extractDownloadCandidate(value: string): string | null {
  const whole = normalizeClipboardText(value);
  if (isDownloadCandidate(whole)) return whole;
  for (const line of value.split(/\r?\n/)) {
    const candidate = normalizeClipboardText(line);
    if (candidate && isDownloadCandidate(candidate)) return candidate;
  }
  return null;
}
