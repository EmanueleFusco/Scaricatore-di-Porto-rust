import { describe, expect, it } from "vitest";
import { redactDetails } from "./diagnostics";
describe("technical error redaction", () => {
  it("hides configured headers, URL credentials, queries and common secret fields", () => {
    const text = "ERROR 403 https://user:fake-password@example.com/video?token=fake-token&id=42\nAuthorization: Bearer fake-auth\nX-Custom: fake-custom\nCookie: session=fake-cookie";
    const safe = redactDetails(text, ["fake-custom"]);
    for (const secret of ["fake-password", "fake-token", "fake-auth", "fake-custom", "fake-cookie"]) expect(safe).not.toContain(secret);
    expect(safe).toContain("ERROR 403");
    expect(safe).toContain("example.com/video");
  });
  it("preserves useful tool errors that contain no credentials", () => {
    expect(redactDetails("ERROR: ffmpeg exited with code 2\nUnsupported codec"))
      .toBe("ERROR: ffmpeg exited with code 2\nUnsupported codec");
  });
});
