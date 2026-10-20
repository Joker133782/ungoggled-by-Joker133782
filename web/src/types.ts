export type Status = {
  version?: string;
  phase: string;
  enabled: boolean;
  hdmi: string;
  hdmi_connected?: boolean;
  usb_state?: string | null;
  bitrate_mbps: number;
  input_width?: number;
  input_height?: number;
  input_fps?: number;
  input_nominal_fps?: number;
  output_fps?: number;
  hdmi_width?: number;
  hdmi_height?: number;
  hdmi_hz?: number;
  hdmi_interlaced?: boolean;
  hdmi_modes?: string[];
  temperature_c?: number;
  uptime_seconds: number;
  video_bytes: number;
  discarded_bytes?: number;
  decoder_starts?: number;
  message?: string;
  display_error?: string;
  beeper_error?: string | null;
  power_button_error?: string | null;
  goggles_led_error?: string | null;
  video_led_error?: string | null;
  hdmi_led_error?: string | null;
  time_unix_ms?: number;
  time?: string;
  timezone?: string;
  time_synced?: boolean;
  fallback_video?: boolean;
  goggles_protocol?: "goggles3" | "goggles2";
  goggles_protocol_state?: "probing" | "video" | "forced";
  disk_warning?: boolean;
};
export type Settings = {
  hdmi_mode: string;
  fallback_image: string | null;
  preview_enabled: boolean;
  beeper_pin: number | null;
  power_button_pin: number | null;
  goggles_led_pin: number | null;
  video_led_pin: number | null;
  hdmi_led_pin: number | null;
  beep_until_time_synced: boolean;
};
export type ImageEntry = {
  id: string;
  url: string;
  kind?: "image" | "video";
};
export type Sample = {
  t: number;
  bitrate_mbps: number | null;
  input_fps: number | null;
  output_fps: number | null;
  temperature_c: number | null;
  phase: string;
  hdmi: string;
};
export type History = { now: number; samples: Sample[] };
export type RecordingStatus = {
  recording: boolean;
  name?: string;
  bytes?: number;
  seconds?: number;
};
export type Recording = {
  name: string;
  size: number;
  modified: number | null;
};
export type UsbVideo = {
  id: string;
  name: string;
  device: string;
  size: number;
  thumbnail: boolean;
};
export type UsbVideoStatus = {
  playing: boolean;
  name?: string;
  paused?: boolean | null;
  position?: number | null;
  duration?: number | null;
};
export type Wifi = {
  available: boolean;
  ssid?: string;
  applying?: boolean;
  error?: string;
};
