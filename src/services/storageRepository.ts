import { invokeNative } from "./nativeBridge";
import { open } from "@tauri-apps/plugin-dialog";

export interface StorageInfo {
  databasePath: string;
  projectsRoot: string;
  configuredProjectsRoot: string;
  defaultProjectsRoot: string;
  restartRequired: boolean;
  configuredRootAvailable: boolean;
  schemaVersion: number;
}

export interface StorageUsage {
  projectBytes: number;
  databaseBytes: number;
  availableBytes: number;
  totalBytes: number;
}

export const storageRepository = {
  info: () => invokeNative<StorageInfo>("get_storage_info"),
  usage: () => invokeNative<StorageUsage>("get_storage_usage"),
  openProjectsRoot: () => invokeNative<void>("open_projects_root"),
  chooseProjectsRoot: async () => {
    const selected = await open({ directory: true, multiple: false, title: "选择知画项目根目录" });
    if (typeof selected !== "string") return undefined;
    return invokeNative<StorageInfo>("set_projects_root", { path: selected });
  },
  resetProjectsRoot: () => invokeNative<StorageInfo>("reset_projects_root"),
};
