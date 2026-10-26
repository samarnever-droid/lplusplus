export interface VoiceSegment {
  id: string;
  text: string;
  start: number;
  end: number;
  duration: number;
  pause_after: number;
}

export interface TimelineData {
  lead_in: number;
  total_duration: number;
  fps: number;
  segments: VoiceSegment[];
  vo_env: number[];
  bg_env: number[];
}

export interface SceneDescriptor {
  id: number;
  code: string;
  title: string;
  subtitle: string;
  start: number;
  end: number;
  accent: string;
  secondary: string;
}

export interface FrameContext {
  ctx: CanvasRenderingContext2D;
  W: number;
  H: number;
  t: number;
  dt: number;
  frame: number;
  voEnergy: number;
  bgEnergy: number;
  scene: SceneDescriptor;
  sceneProgress: number;
  sceneLocalTime: number;
  activeSegment: VoiceSegment | null;
}
