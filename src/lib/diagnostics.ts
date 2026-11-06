/** Remove credentials from technical details before showing or copying them. */
export function redactDetails(text: string, sensitiveValues: readonly string[] = []): string {
  let safe = text;
  for (const secret of sensitiveValues) {
    if (secret.trim()) safe = safe.replaceAll(secret, "[redacted]");
  }
  safe = safe.replace(/https?:\/\/[^\s<>"']+/gi, (raw) => {
    try {
      const url = new URL(raw);
      if (url.username || url.password) { url.username = "redacted"; url.password = ""; }
      for (const key of new Set(url.searchParams.keys())) url.searchParams.set(key, "redacted");
      url.hash = "";
      return url.toString();
    } catch { return "[redacted URL]"; }
  });
  return safe.replace(/\b((?:authorization|proxy-authorization|cookie|set-cookie|x-api-key|api[_-]?key|access[_-]?token|refresh[_-]?token|password|secret)\s*[:=]\s*)[^\r\n]+/gi, "$1[redacted]");
}
