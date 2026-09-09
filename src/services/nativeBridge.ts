import { invoke, isTauri } from "@tauri-apps/api/core";

export async function invokeOptional<T>(command: string, args?: Record<string, unknown>): Promise<T | undefined> {
  if (!isTauri()) return undefined;

  try {
    return await invoke<T>(command, args);
  } catch (error) {
    console.info(`[知画] ${command} 尚未由桌面端实现，使用本地回退。`, error);
    return undefined;
  }
}

export async function invokeNative<T>(command: string, args?: Record<string, unknown>): Promise<T | undefined> {
  if (!isTauri()) return undefined;
  return invoke<T>(command, args);
}

export function isNativeRuntime(): boolean {
  return isTauri();
}

export function readLocal<T>(key: string, fallback: T): T {
  try {
    const value = localStorage.getItem(key);
    return value ? (JSON.parse(value) as T) : fallback;
  } catch {
    return fallback;
  }
}

export function writeLocal<T>(key: string, value: T): void {
  localStorage.setItem(key, JSON.stringify(value));
}
