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
  /** While a USB video plays (else null): clip seconds per second of real
   * time (1 = real time), new pictures per second on the screen, and the
   * clip's own frame rate. */
  usb_speed?: number | null;
  usb_fps?: number | null;
  usb_clip_fps?: number | null;
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
  /** An update changed boot settings that only take effect after a reboot. */
  reboot_required?: boolean;
};
export type HdmiConnector = {
  id: number;
  name: string;
  connected: boolean;
};
export type Settings = {
  hdmi_mode: string;
  hdmi_connector: number | null;
  fallback_image: string | null;
  preview_enabled: boolean;
  beeper_pin: number | null;
  power_button_pin: number | null;
  goggles_led_pin: number | null;
  video_led_pin: number | null;
  hdmi_led_pin: number | null;
  beep_until_time_synced: boolean;
  stream_enabled: boolean;
  stream_quality: string;
  auto_record: boolean;
  crash_buffer: boolean;
  record_to_usb: boolean;
  wifi_always_ap: boolean;
  terminal_enabled: boolean;
  stretch_live: boolean;
  stretch_usb: boolean;
};
/** One entry of the idle-screen library (`GET /api/images`). */
export type ImageEntry = {
  id: string;
  url: string;
  /** A small JPEG: a scaled-down image, or a video's poster frame. */
  thumbnail: string;
  kind: "image" | "video";
  name: string;
  /** Shipped with ungoggled: always there, can't be deleted. */
  builtin: boolean;
  size: number;
  modified: number;
  width: number | null;
  height: number | null;
  fps: number | null;
};
export type ImageLimits = {
  videos: { used: number; max: number };
  images: { used: number; max: number };
};
export type ImageLibrary = { images: ImageEntry[]; limits: ImageLimits };
export type Sample = {
  t: number;
  bitrate_mbps: number | null;
  input_fps: number | null;
  output_fps: number | null;
  temperature_c: number | null;
  phase: string;
  hdmi: string;
  cpu_percent: number;
  ram_used_mib: number;
  ram_total_mib: number;
  disk_read_kib_s: number;
  disk_write_kib_s: number;
  wifi_rx_mbps: number;
  wifi_tx_mbps: number;
  lan_rx_mbps: number;
  lan_tx_mbps: number;
};
export type History = {
  /** Identifies one run of the service; `t` restarts with every run. */
  instance: number;
  now: number;
  samples: Sample[];
};
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
  thumbnail: boolean;
};
export type UsbVideo = {
  id: string;
  name: string;
  device: string;
  size: number;
  /** Unix seconds; absent if the file's modification time couldn't be read. */
  mtime?: number;
  thumbnail: boolean;
};
export type UsbVideoStatus = {
  playing: boolean;
  name?: string;
  paused?: boolean | null;
  position?: number | null;
  duration?: number | null;
  drives: string[];
  scan: { done: number; total: number } | null;
};
export type Wifi = {
  available: boolean;
  ssid?: string;
  applying?: boolean;
  error?: string;
};
export type WifiNetwork = { ssid: string; signal: number; secured: boolean };
/** `GET /api/wifi/share`: what a spectator's phone needs to join. */
export type WifiShare =
  | {
      mode: "ap";
      ssid: string | null;
      security: "WPA" | "nopass";
      password: string | null;
    }
  /** Joined another network; the Pi's own access point is down. */
  | { mode: "client"; ssid: string | null }
  | { mode: "none" };
export type TerminalStatus = {
  enabled: boolean;
  /** Someone (maybe this browser) has a terminal open right now. */
  active: boolean;
  idle_seconds: number;
  max_seconds: number;
};
export type WifiClient = {
  active: boolean;
  ssid?: string | null;
  ip?: string | null;
  applying?: boolean;
  error?: string | null;
};
