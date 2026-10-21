/** 把 Rust 的 `std::env::consts::OS` 值转成展示名。 */
export function platformLabel(platform?: string): string {
  if (!platform) return "—";
  switch (platform) {
    case "windows":
      return "Windows";
    case "macos":
      return "macOS";
    case "linux":
      return "Linux";
    case "android":
      return "Android";
    case "ios":
      return "iOS";
    default:
      return platform;
  }
}
