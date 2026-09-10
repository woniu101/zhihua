import type {
  GenerationJob,
  RuntimeCapabilities,
  VideoGenerationRequest,
  VideoProvider,
} from "../domain/providers";
import { invokeNative } from "./nativeBridge";

export interface ServiceConnectionResult {
  baseUrl: string;
  service: string;
  version: string;
}

export interface ServiceConnectionInfo {
  configured: boolean;
  instanceId?: string;
  baseUrl?: string;
  credentialStored: boolean;
}

export interface ServiceProbe {
  reachable: boolean;
  compatible: boolean;
  serviceVersion: string;
  apiVersion?: string;
  workflowManifestVersion: string;
  modelManifestVersion: string;
  comfyuiConnected: boolean;
  comfyuiReady: boolean;
  queueActive: number;
  queueQueued: number;
  detail: string;
}

export interface ServiceConnectionFailure {
  code: string;
  message: string;
}

interface NativeServiceJob {
  id: string;
  clientRequestId: string;
  promptId?: string;
  status: GenerationJob["status"];
  progress: number;
  errorCode?: string;
  errorMessage?: string;
  createdAt: string;
  updatedAt: string;
}

function mapJob(job: NativeServiceJob): GenerationJob {
  return {
    id: job.id,
    clientRequestId: job.clientRequestId,
    remotePromptId: job.promptId,
    status: job.status,
    progress: job.progress,
    stageMessage: job.errorMessage ?? job.status,
    createdAt: job.createdAt,
    updatedAt: job.updatedAt,
    errorCode: job.errorCode,
    errorMessage: job.errorMessage,
  };
}

function workflowFor(request: VideoGenerationRequest): string {
  const family = {
    t2v: "t2v",
    i2v: "i2v",
    flf2v: "flf2v",
    r2v: "ref2va",
    continue: "i2v",
  }[request.mode];
  const quality = request.quality === "fast" ? "turbo" : "high";
  return `h3-${family}-${quality}-v1`;
}

export async function testServiceConnection(
  baseUrl: string,
): Promise<ServiceConnectionResult> {
  const result = await invokeNative<ServiceConnectionResult>(
    "test_service_connection",
    { baseUrl },
  );
  if (!result) throw new Error("请在知画桌面客户端中测试服务连接。");
  return result;
}

export function normalizeConnectionFailure(
  error: unknown,
): ServiceConnectionFailure {
  if (typeof error === "object" && error !== null) {
    const candidate = error as Partial<ServiceConnectionFailure>;
    if (typeof candidate.message === "string") {
      return {
        code: typeof candidate.code === "string" ? candidate.code : "unknown",
        message: candidate.message,
      };
    }
  }
  if (error instanceof Error)
    return { code: "unknown", message: error.message };
  if (typeof error === "string") return { code: "unknown", message: error };
  return { code: "unknown", message: "知画服务连接失败。" };
}

export const serviceRepository = {
  info: () =>
    invokeNative<ServiceConnectionInfo>("get_service_connection_info"),
  save: (input: { instanceId: string; baseUrl: string; token: string }) =>
    invokeNative<ServiceConnectionInfo>("save_service_connection", { input }),
  clear: () => invokeNative<void>("clear_service_connection"),
  probe: () => invokeNative<ServiceProbe>("probe_service"),
};

export class ComfyUiH3Provider implements VideoProvider {
  private capabilities?: RuntimeCapabilities;

  async getCapabilities(): Promise<RuntimeCapabilities> {
    if (this.capabilities) return this.capabilities;
    const probe = await serviceRepository.probe();
    if (!probe) throw new Error("知画服务仅可在桌面客户端中使用");
    if (!probe.compatible) throw new Error(probe.detail);
    this.capabilities = {
      serviceVersion: probe.serviceVersion,
      apiVersion: probe.apiVersion ?? "v1",
      workflowVersion: probe.workflowManifestVersion,
      modelManifestVersion: probe.modelManifestVersion,
      comfyUiReady: probe.comfyuiReady,
      workflows: ["t2v", "i2v", "flf2v", "r2v", "seedvr2"],
    };
    return this.capabilities;
  }

  async submit(request: VideoGenerationRequest): Promise<GenerationJob> {
    const job = await invokeNative<NativeServiceJob>("submit_service_job", {
      input: {
        clientRequestId: request.clientRequestId,
        projectId: request.projectId,
        sceneId: request.sceneId,
        kind:
          request.mode === "r2v"
            ? "video_reference_remake"
            : "video_candidate",
        workflowId: workflowFor(request),
        parameters: {
          mode: request.mode,
          quality: request.quality,
          aspectRatio: request.aspectRatio,
          durationSec: request.durationSec,
          prompt: request.prompt,
          narration: request.narration,
          seed: request.seed,
          assetIds: request.assetIds,
          discardH3Audio: request.discardH3Audio,
        },
      },
    });
    if (!job) throw new Error("知画服务仅可在桌面客户端中使用");
    return mapJob(job);
  }

  async getStatus(jobId: string): Promise<GenerationJob> {
    const job = await invokeNative<NativeServiceJob>("get_service_job", {
      jobId,
    });
    if (!job) throw new Error("知画服务仅可在桌面客户端中使用");
    return mapJob(job);
  }

  async cancel(jobId: string): Promise<void> {
    await invokeNative("cancel_service_job", { jobId });
  }

  async downloadResult(): Promise<string> {
    throw new Error("结果下载将在工作流执行器完成后开放");
  }
}
