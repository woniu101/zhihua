import type { SceneDraft } from "../domain/storyboard";
import {
  invokeNative,
  isNativeRuntime,
  readLocal,
  writeLocal,
} from "./nativeBridge";

function storageKey(projectId: string): string {
  return `zhihua.storyboard.${projectId}.v1`;
}

function localList(projectId: string): SceneDraft[] {
  return readLocal<SceneDraft[]>(storageKey(projectId), []);
}

function persistLocal(projectId: string, scenes: SceneDraft[]): void {
  writeLocal(storageKey(projectId), scenes);
}

export function activeProjectId(): string | undefined {
  return readLocal<string | undefined>("zhihua.activeProjectId", undefined);
}

export const storyboardRepository = {
  async list(projectId: string): Promise<SceneDraft[]> {
    const native = await invokeNative<SceneDraft[]>("list_storyboard_scenes", {
      projectId,
    });
    return native ?? localList(projectId);
  },

  async upsert(scene: SceneDraft): Promise<SceneDraft> {
    const native = await invokeNative<SceneDraft>("upsert_storyboard_scene", {
      scene,
    });
    if (native) return native;
    const next = [
      ...localList(scene.projectId).filter((item) => item.id !== scene.id),
      scene,
    ].sort((left, right) => left.order - right.order);
    persistLocal(scene.projectId, next);
    return scene;
  },

  async remove(projectId: string, id: string): Promise<void> {
    if (isNativeRuntime()) {
      await invokeNative<void>("delete_storyboard_scene", { projectId, id });
      return;
    }
    persistLocal(
      projectId,
      localList(projectId).filter((item) => item.id !== id),
    );
  },

  async reorder(projectId: string, orderedSceneIds: string[]): Promise<void> {
    if (isNativeRuntime()) {
      await invokeNative<void>("reorder_storyboard_scenes", {
        input: { projectId, orderedSceneIds },
      });
      return;
    }
    const byId = new Map(localList(projectId).map((scene) => [scene.id, scene]));
    const next = orderedSceneIds
      .map((id, order) => {
        const scene = byId.get(id);
        return scene ? { ...scene, order } : undefined;
      })
      .filter((scene): scene is SceneDraft => Boolean(scene));
    persistLocal(projectId, next);
  },
};
