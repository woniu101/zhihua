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
  workflows: string[];
  availableWorkflows: string[];
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

function candidateDimensions(aspectRatio: VideoGenerationRequest["aspectRatio"]): {
  width: number;
  height: number;
} {
  return {
    auto: { width: 1344, height: 768 },
    "16:9": { width: 1344, height: 768 },
    "9:16": { width: 768, height: 1344 },
    "4:3": { width: 1152, height: 864 },
    "3:4": { width: 864, height: 1152 },
    "1:1": { width: 1024, height: 1024 },
  }[aspectRatio];
}

function frameLength(durationSec: 5 | 10 | 15): 124 | 243 | 362 {
  return { 5: 124, 10: 243, 15: 362 }[durationSec] as 124 | 243 | 362;
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
  async getCapabilities(): Promise<RuntimeCapabilities> {
    const probe = await serviceRepository.probe();
    if (!probe) throw new Error("知画服务仅可在桌面客户端中使用");
    if (!probe.compatible) throw new Error(probe.detail);
    return {
      serviceVersion: probe.serviceVersion,
      apiVersion: probe.apiVersion ?? "v1",
      workflowVersion: probe.workflowManifestVersion,
      modelManifestVersion: probe.modelManifestVersion,
      comfyUiReady: probe.comfyuiReady,
      acceptedWorkflowIds: probe.workflows,
      availableWorkflowIds: probe.availableWorkflows,
      workflows: [
        probe.availableWorkflows.some((item) => item.startsWith("h3-t2v-")) && "t2v",
        probe.availableWorkflows.some((item) => item.startsWith("h3-i2v-")) && "i2v",
        probe.availableWorkflows.some((item) => item.startsWith("h3-flf2v-")) && "flf2v",
        probe.availableWorkflows.some((item) => item.startsWith("h3-ref2va-")) && "r2v",
        probe.availableWorkflows.some((item) => item.startsWith("seedvr2-")) && "seedvr2",
      ].filter((item): item is RuntimeCapabilities["workflows"][number] => Boolean(item)),
    };
  }

  async submit(request: VideoGenerationRequest): Promise<GenerationJob> {
    const capabilities = await this.getCapabilities();
    const workflowId = workflowFor(request);
    if (!capabilities.acceptedWorkflowIds?.includes(workflowId)) {
      throw new Error(`知画服务不接受工作流 ${workflowId}，请先更新服务。`);
    }
    if (!capabilities.availableWorkflowIds?.includes(workflowId)) {
      throw new Error(`工作流 ${workflowId} 尚未安装，当前不会启动 GPU。`);
    }
    if (request.mode !== "t2v") {
      throw new Error("当前分镜需要先把参考素材上传到知画服务；素材传输完成前不会启动 GPU。");
    }
    const dimensions = candidateDimensions(request.aspectRatio);
    const job = await invokeNative<NativeServiceJob>("submit_service_job", {
      input: {
        clientRequestId: request.clientRequestId,
        projectId: request.projectId,
        sceneId: request.sceneId,
        kind: "video_candidate",
        workflowId,
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
          width: dimensions.width,
          height: dimensions.height,
          length: frameLength(request.durationSec),
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
