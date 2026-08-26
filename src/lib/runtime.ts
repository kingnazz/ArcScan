// Which edition of ArcScan this is, and where it keeps its data.
//
// The backend decides all of this and the interface only displays it. There is
// deliberately no path arithmetic here and no way to ask for a different answer:
// `runtime_info` reports the data root the Rust startup already resolved, as a
// single display string, and the two actions that use it take no argument at all
// (`open_data_folder` opens that root; `open_portable_downloads` opens a fixed
// URL). A frontend that could build a portable path could get it wrong, and a
// wrong answer here would be a wrong answer about where somebody's scan history
// is.

export type Edition = "installed" | "portable";
export type UpdaterMode = "installer" | "manual";

export interface RuntimeInfo {
  edition: Edition;
  version: string;
  /** "x64", "ARM64", "x86" — the build's target, not the host's. */
  architecture: string;
  /** The data root, already formatted for display and for Copy data path. */
  data_root: string;
  /** Whether the startup write probe succeeded. */
  writable: boolean;
  updater_mode: UpdaterMode;
}

export const isPortable = (info: RuntimeInfo | null): boolean => info?.edition === "portable";

/**
 * How the edition is described in Settings: "Portable edition · Windows x64".
 *
 * The platform word comes from the runtime's architecture label rather than from
 * the user agent, because the point of the line is to tell an operator which
 * *build* they are running — which is exactly the thing a user agent gets wrong
 * for an x64 build on an ARM64 machine.
 */
export function editionLabel(info: RuntimeInfo, platform: string): string {
  const edition = info.edition === "portable" ? "Portable edition" : "Installed edition";
  return `${edition} · ${platform} ${info.architecture}`;
}

/**
 * What to tell a portable operator when a newer version exists.
 *
 * Not "Update now". A portable copy is a folder somebody chose to put
 * somewhere, and replacing the application files inside it while keeping
 * ArcScanData is their deliberate act, not something an installer should do
 * behind them. The wording says what to do and, importantly, what to keep.
 */
export const PORTABLE_UPDATE_STEPS =
  "You're using ArcScan Portable. Download the new Portable ZIP, close ArcScan, " +
  "replace the application files, and keep the ArcScanData folder.";
