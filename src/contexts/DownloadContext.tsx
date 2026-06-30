import { listen } from "@tauri-apps/api/event";
import { type ComponentChildren, createContext } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { getResumeInfo } from "../lib/GameDownloader";
import type {
	ProtonSetupProgress,
	ResumeInfo,
	SophonProgress,
	Variants,
} from "../types";

export interface DownloadState {
	isPaused: boolean;
	isDownloading: boolean;
	isAssembling: boolean;
	isVerifying: boolean;
	isFetchingManifest: boolean;
	isCalculatingDownloads: boolean;
	isInstallingPlugins: boolean;
	isApplyingPreinstall: boolean;
	isError: boolean;
	isFinished: boolean;
	downloadingGame: Variants | null;
	downloadedBytes: number;
	downloadTotal: number;
	speedBps: number;
	etaSeconds: number;
	assembledFiles: number;
	totalFiles: number;
	checkedFiles: number;
	scannedFiles: number;
	errorCount: number;
	errorMessage: string;
	warningMessage: string;
	pluginName: string;
	pluginProgress: string;
	isResumable: boolean;
	resumeInfo: ResumeInfo | null;
	isSettingUpProton: boolean;
	protonSetupComponent: string;
	protonSetupPhase: string;
	protonSetupDownloadedBytes: number;
	protonSetupDownloadTotal: number;
}

interface DownloadContextType {
	state: DownloadState;
	setDownloadingGame: (game: Variants | null) => void;
	setResumable: (info: ResumeInfo | null) => void;
	setProtonSetupProgress: (event: ProtonSetupProgress) => void;
}

const initialState: DownloadState = {
	isPaused: false,
	isDownloading: false,
	isAssembling: false,
	isVerifying: false,
	isFetchingManifest: false,
	isCalculatingDownloads: false,
	isInstallingPlugins: false,
	isApplyingPreinstall: false,
	isError: false,
	isFinished: false,
	downloadingGame: null,
	downloadedBytes: 0,
	downloadTotal: 0,
	speedBps: 0,
	etaSeconds: 0,
	assembledFiles: 0,
	totalFiles: 0,
	checkedFiles: 0,
	scannedFiles: 0,
	errorCount: 0,
	errorMessage: "",
	warningMessage: "",
	pluginName: "",
	pluginProgress: "",
	isResumable: false,
	resumeInfo: null,
	isSettingUpProton: false,
	protonSetupComponent: "",
	protonSetupPhase: "",
	protonSetupDownloadedBytes: 0,
	protonSetupDownloadTotal: 0,
};

export const DownloadContext = createContext<DownloadContextType>({
	state: initialState,
	setDownloadingGame: () => {},
	setResumable: () => {},
	setProtonSetupProgress: () => {},
});

export const DownloadProvider = ({
	children,
}: {
	children: ComponentChildren;
}) => {
	const [state, setState] = useState<DownloadState>({ ...initialState });
	const downloadingGameRef = useRef<Variants | null>(null);

	const setDownloadingGame = (game: Variants | null) => {
		downloadingGameRef.current = game;
		setState((prev) => ({ ...prev, downloadingGame: game }));
	};

	const setResumable = (info: ResumeInfo | null) => {
		setState((prev) => ({
			...prev,
			isResumable: info !== null,
			resumeInfo: info,
		}));
	};

	useEffect(() => {
		getResumeInfo().then((info) => {
			if (info) {
				setResumable(info);
			}
		});
	}, []);

	useEffect(() => {
		const unlisten = listen("sophon://progress", (event) => {
			const payload = event.payload as SophonProgress;

			setState((prev) => {
				switch (payload.type) {
					case "fetchingManifest":
						return {
							...prev,
							isFetchingManifest: true,
							isCalculatingDownloads: false,
							isPaused: false,
							isDownloading: false,
							isAssembling: false,
							isVerifying: false,
							isError: false,
							isFinished: false,
							isResumable: false,
							resumeInfo: null,
							downloadingGame:
								prev.downloadingGame ?? downloadingGameRef.current,
							errorMessage: "",
							warningMessage: "",
						};
					case "calculatingDownloads":
						return {
							...prev,
							isFetchingManifest: false,
							isCalculatingDownloads: true,
							isPaused: false,
							isDownloading: false,
							isAssembling: false,
							isVerifying: false,
							isError: false,
							isFinished: false,
							downloadingGame:
								prev.downloadingGame ?? downloadingGameRef.current,
							checkedFiles: payload.checkedFiles,
							totalFiles: payload.totalFiles,
						};
					case "downloading":
						return {
							...prev,
							isDownloading: true,
							isPaused: false,
							isFetchingManifest: false,
							isCalculatingDownloads: false,
							downloadingGame:
								prev.downloadingGame ?? downloadingGameRef.current,
							downloadedBytes: payload.downloadedBytes,
							downloadTotal: payload.totalBytes,
							speedBps: payload.speedBps,
							etaSeconds: payload.etaSeconds,
						};
					case "paused":
						return {
							...prev,
							isPaused: true,
							downloadedBytes: payload.downloadedBytes,
							downloadTotal: payload.totalBytes,
							speedBps: 0,
							etaSeconds: 0,
						};
					case "assembling":
						return {
							...prev,
							isAssembling: true,
							isFetchingManifest: false,
							assembledFiles: payload.assembledFiles,
							totalFiles: payload.totalFiles,
						};
					case "verifying":
						return {
							...prev,
							isVerifying: true,
							isPaused: false,
							isDownloading: false,
							isAssembling: false,
							isFetchingManifest: false,
							isError: false,
							isFinished: false,
							scannedFiles: payload.scannedFiles,
							totalFiles: payload.totalFiles,
							errorCount: payload.errorCount,
						};
					case "warning":
						return {
							...prev,
							warningMessage: payload.message,
						};
					case "error":
						return {
							...prev,
							isError: true,
							isPaused: false,
							isDownloading: false,
							isAssembling: false,
							isFetchingManifest: false,
							isVerifying: false,
							isInstallingPlugins: false,
							errorMessage: payload.message,
						};
					case "installingPlugins":
						return {
							...prev,
							isInstallingPlugins: true,
							isDownloading: false,
							isAssembling: false,
							isVerifying: false,
							isFetchingManifest: false,
							pluginName: payload.currentPlugin,
							pluginProgress: `Installing plugins: ${payload.currentPlugin} (${payload.totalPlugins})`,
						};
					case "downloadingPlugin":
						return {
							...prev,
							isInstallingPlugins: true,
							isDownloading: true,
							isAssembling: false,
							isVerifying: false,
							isFetchingManifest: false,
							pluginName: payload.name,
							downloadedBytes: payload.downloadedBytes,
							downloadTotal: payload.totalBytes,
							pluginProgress: `Downloading plugin: ${payload.name}`,
						};
					case "applyingPreinstall":
						return {
							...prev,
							isApplyingPreinstall: true,
							isDownloading: false,
							isAssembling: false,
							isVerifying: false,
							isFetchingManifest: false,
							isPaused: false,
							isError: false,
							isFinished: false,
							assembledFiles: payload.appliedFiles,
							totalFiles: payload.totalFiles,
						};
					case "finished":
						return {
							...initialState,
							isFinished: true,
						};
				}
			});
		});

		return () => {
			unlisten.then((fn) => fn());
		};
	}, []);

	const setProtonSetupProgress = (event: ProtonSetupProgress) => {
		setState((prev) => {
			switch (event.type) {
				case "protonSetupDownloading":
					return {
						...prev,
						isSettingUpProton: true,
						protonSetupComponent: event.component,
						protonSetupPhase: "downloading",
						protonSetupDownloadedBytes: event.downloaded_bytes,
						protonSetupDownloadTotal: event.total_bytes,
					};
				case "protonSetupExtracting":
					return {
						...prev,
						isSettingUpProton: true,
						protonSetupComponent: event.component,
						protonSetupPhase: "extracting",
						protonSetupDownloadedBytes: 0,
						protonSetupDownloadTotal: 0,
					};
				case "protonSetupInstalling":
					return {
						...prev,
						isSettingUpProton: true,
						protonSetupComponent: event.component,
						protonSetupPhase: "installing",
						protonSetupDownloadedBytes: 0,
						protonSetupDownloadTotal: 0,
					};
				case "protonSetupFinished":
					return {
						...prev,
						isSettingUpProton: false,
						protonSetupComponent: "",
						protonSetupPhase: "",
						protonSetupDownloadedBytes: 0,
						protonSetupDownloadTotal: 0,
					};
			}
		});
	};

	return (
		<DownloadContext.Provider
			value={{
				state,
				setDownloadingGame,
				setResumable,
				setProtonSetupProgress,
			}}
		>
			{children}
		</DownloadContext.Provider>
	);
};
