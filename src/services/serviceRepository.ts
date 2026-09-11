import type {
  GenerationJob,
  RuntimeCapabilities,
  VideoGenerationRequest,
  VideoProvider,
} from "../domain/providers";
import { invokeNative } from "./nativeBridge";
import { assetRepository } from "./assetRepository";
import { currentAssetVersion } from "../domain/assets";

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
  statusDetail?: string;
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
    stageMessage: job.errorMessage ?? job.statusDetail ?? job.status,
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

export interface LocalGenerationJob {
  clientRequestId: string;
  remoteJobId?: string;
  projectId: string;
  sceneId: string;
  kind: string;
  workflowId: string;
  status: string;
  progress: number;
  workerId?: string;
  leaseExpiresAt?: string;
  attempt: number;
  errorCode?: string;
  errorMessage?: string;
  createdAt: string;
  updatedAt: string;
}

export interface ComputePoolPlanInput {
  taskCount: number;
  policy: {
    mode: "single" | "elastic";
    maxWorkers: number;
    maxInstances: number;
  };
  readyWorkers: number;
  activeInstances: number;
  accountInstanceQuotaRemaining: number;
  capacityAvailableInstances: number;
  affordableNewInstances: number;
  workersPerNewInstance: number;
  hourlyCostMinorPerNewInstance?: number;
  estimatedRuntimeSeconds?: number;
  currency?: string;
}

export interface ComputePoolPlan {
  taskCount: number;
  plannedWorkers: number;
  reusableWorkers: number;
  instancesToCreate: number;
  queuedTasks: number;
  estimatedNewInstanceCostMinor?: number;
  currency?: string;
  requiresConfirmation: boolean;
  limitingFactors: string[];
}

export interface ServiceInputUpload {
  inputId: string;
  remoteFile: string;
  originalFilename: string;
  mediaType: string;
  sizeBytes: number;
  sha256: string;
}

export interface ServiceArtifactDownload {
  destinationPath: string;
  sizeBytes: number;
  sha256: string;
  resumed: boolean;
}

export interface VideoUpscaleRequest {
  clientRequestId: string;
  projectId: string;
  sceneId: string;
  sourcePath: string;
  seed: number;
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
  prepareGeneration: () =>
    invokeNative<ServiceProbe>("prepare_generation_service"),
  listLocalJobs: (projectId?: string) =>
    invokeNative<LocalGenerationJob[]>("list_local_jobs", { projectId }),
  planComputePool: (input: ComputePoolPlanInput) =>
    invokeNative<ComputePoolPlan>("plan_generation_compute_pool", { input }),
  uploadInput: (sourcePath: string) =>
    invokeNative<ServiceInputUpload>("upload_service_input", { sourcePath }),
  deleteInput: (inputId: string) =>
    invokeNative<void>("delete_service_input", { inputId }),
  downloadArtifact: (input: {
    jobId: string;
    artifactId: string;
    destinationPath: string;
    expectedSizeBytes: number;
    expectedSha256: string;
  }) => invokeNative<ServiceArtifactDownload>("download_service_artifact", { input }),
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
    const uploaded: ServiceInputUpload[] = [];
    const parameters: Record<string, unknown> = {};
    if (request.mode !== "t2v") {
      const assets = await assetRepository.list(request.projectId);
      const selected = request.assetIds
        .map((id) => assets.find((asset) => asset.id === id))
        .filter((asset): asset is NonNullable<typeof asset> => Boolean(asset));
      const upload = async (asset: (typeof selected)[number]) => {
        const path = currentAssetVersion(asset).storedPath;
        if (!path) throw new Error(`参考素材“${asset.name}”没有可上传的本地文件。`);
        const result = await serviceRepository.uploadInput(path);
        if (!result) throw new Error(`参考素材“${asset.name}”上传失败。`);
        uploaded.push(result);
        return result.remoteFile;
      };
      try {
        if (request.mode === "i2v" || request.mode === "continue") {
          const image = selected.find((asset) => asset.mediaType === "image");
          if (!image) throw new Error("“从这张画面开始”需要选择一张首帧图片。");
          parameters.firstFrameFile = await upload(image);
        } else if (request.mode === "flf2v") {
          const images = selected.filter((asset) => asset.mediaType === "image");
          if (images.length < 2) throw new Error("“首尾画面过渡”需要按顺序选择首帧和尾帧图片。");
          parameters.firstFrameFile = await upload(images[0]);
          parameters.lastFrameFile = await upload(images[1]);
        } else if (request.mode === "r2v") {
          const video = selected.find((asset) => asset.mediaType === "video");
          const image = selected.find((asset) => asset.mediaType === "image");
          if (!video || !image) throw new Error("“保持角色与场景”需要一段参考视频和一张参考图片。");
          parameters.referenceVideoFile = await upload(video);
          parameters.referenceImageFile = await upload(image);
        }
      } catch (error) {
        await Promise.allSettled(uploaded.map((item) => serviceRepository.deleteInput(item.inputId)));
        throw error;
      }
    }
    const dimensions = candidateDimensions(request.aspectRatio);
    try {
      const probe = await serviceRepository.prepareGeneration();
      if (!probe?.comfyuiReady) {
        throw new Error(probe?.detail ?? "生成环境尚未就绪，请稍后重试。");
      }
      const job = await invokeNative<NativeServiceJob>("submit_service_job", {
        input: {
          clientRequestId: request.clientRequestId,
          projectId: request.projectId,
          sceneId: request.sceneId,
          kind: "video_candidate",
          workflowId,
          parameters: {
            ...parameters,
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
    } catch (error) {
      await Promise.allSettled(uploaded.map((item) => serviceRepository.deleteInput(item.inputId)));
      throw error;
    }
  }

  async submitUpscale(request: VideoUpscaleRequest): Promise<GenerationJob> {
    const workflowId = "seedvr2-1080p-v1";
    const capabilities = await this.getCapabilities();
    if (!capabilities.acceptedWorkflowIds?.includes(workflowId)) {
      throw new Error("知画服务版本尚未接受 SeedVR2 1080p 工作流，请先更新服务。");
    }
    if (!capabilities.availableWorkflowIds?.includes(workflowId)) {
      throw new Error("SeedVR2 1080p 工作流或公共模型尚未就绪，当前不会启动 GPU。");
    }
    const uploaded = await serviceRepository.uploadInput(request.sourcePath);
    if (!uploaded) throw new Error("正式版本上传失败。");
    try {
      const probe = await serviceRepository.prepareGeneration();
      if (!probe?.comfyuiReady) {
        throw new Error(probe?.detail ?? "1080p 生成环境尚未就绪，请稍后重试。");
      }
      const job = await invokeNative<NativeServiceJob>("submit_service_job", {
        input: {
          clientRequestId: request.clientRequestId,
          projectId: request.projectId,
          sceneId: request.sceneId,
          kind: "video_upscale",
          workflowId,
          parameters: {
            sourceVideoFile: uploaded.remoteFile,
            seed: request.seed,
          },
        },
      });
      if (!job) throw new Error("知画服务仅可在桌面客户端中使用");
      return mapJob(job);
    } catch (error) {
      await serviceRepository.deleteInput(uploaded.inputId).catch(() => undefined);
      throw error;
    }
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
