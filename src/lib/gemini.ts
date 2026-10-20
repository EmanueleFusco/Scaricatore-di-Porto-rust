/**
 * Gemini API client types and IPC commands for settings and UI (spec §10.3, §10.5).
 */
import { invoke } from "@tauri-apps/api/core";

export type KeySource = "environment" | "keyring" | "file";

export interface GeminiKeyStatus {
  hasKey: boolean;
  source: KeySource | null;
}

export interface GeminiModelInfo {
  id: string;
  displayName: string;
  description: string;
}

export const FALLBACK_GEMINI_MODELS: GeminiModelInfo[] = [
  {
    id: "gemini-flash-latest",
    displayName: "Gemini Flash (Latest)",
    description: "Recommended general-purpose fast model alias",
  },
  {
    id: "gemini-flash-lite-latest",
    displayName: "Gemini Flash Lite (Latest)",
    description: "Lightweight fast model alias",
  },
];

export async function getGeminiKeyStatus(): Promise<GeminiKeyStatus> {
  return await invoke<GeminiKeyStatus>("gemini_key_get_status");
}

export async function saveGeminiKey(key: string): Promise<KeySource> {
  return await invoke<KeySource>("gemini_key_save", { key });
}

export async function deleteGeminiKey(): Promise<void> {
  await invoke<void>("gemini_key_delete");
}

export async function testGeminiKey(key?: string): Promise<boolean> {
  return await invoke<boolean>("gemini_key_test", { key: key ?? null });
}

export async function listGeminiModels(): Promise<GeminiModelInfo[]> {
  return await invoke<GeminiModelInfo[]>("gemini_models_list");
}
