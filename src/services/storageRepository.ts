import { invokeNative } from "./nativeBridge";

export interface StorageInfo {
  databasePath: string;
  projectsRoot: string;
  schemaVersion: number;
}

export const storageRepository = {
  info: () => invokeNative<StorageInfo>("get_storage_info"),
  openProjectsRoot: () => invokeNative<void>("open_projects_root"),
};
