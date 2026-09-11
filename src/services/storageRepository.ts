import { invokeNative } from "./nativeBridge";

export interface StorageInfo {
  databasePath: string;
  projectsRoot: string;
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
};
