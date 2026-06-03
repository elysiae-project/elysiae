import { invoke } from "@tauri-apps/api/core";
import {
	restoreStateCurrent,
	StateFlags,
} from "@tauri-apps/plugin-window-state";
import { cva } from "class-variance-authority";
import { Settings } from "lucide-preact";
import { useEffect, useRef } from "preact/hooks";
import "./App.css";
import Background from "./components/app/Background.tsx";
import DownloadProgress from "./components/app/DownloadProgress.tsx";
import InstallerButton from "./components/app/InstallerButton.tsx";
import { PhotosensitivityModal } from "./components/app/PhotosensitivityModal.tsx";
import PreinstallButton from "./components/app/PreinstallButton.tsx";
import SettingsModal from "./components/app/SettingsModal.tsx";
import Sidebar from "./components/app/Sidebar.tsx";
import Button from "./components/Button.tsx";
import Titlebar from "./components/Titlebar.tsx";
import { ApiProvider } from "./contexts/ApiContext.tsx";
import { BackgroundProvider } from "./contexts/BackgroundContext.tsx";
import { DownloadProvider } from "./contexts/DownloadContext.tsx";
import { GameProvider } from "./contexts/GameContext.tsx";
import { useApi } from "./hooks/useApi.ts";
import { useGame } from "./hooks/useGame.ts";
import { type ModalHandle, Variants } from "./types";

const textTheme = cva(null, {
	variants: {
		game: {
			[Variants.BH3]: "font-bh3-hkrpg text-white",
			[Variants.HK4E]: "font-hk4e text-black",
			[Variants.HKRPG]: "font-bh3-hkrpg text-black",
			[Variants.NAP]: "font-nap text-white",
		},
	},
});

const bgTheme = cva("h-full w-full overflow-hidden", {
	variants: {
		game: {
			[Variants.BH3]: "bg-bh3-bg rounded-b-xl",
			[Variants.HK4E]: "bg-hk4e-bg",
			[Variants.HKRPG]: "bg-hkrpg-bg rounded-b-xs",
			[Variants.NAP]: "bg-nap-bg rounded-br-xl border-nap-border",
		},
	},
});

const App = () => {
	const { game } = useGame();
	const { graphics } = useApi();
	const settingsModal = useRef<ModalHandle>(null);

	useEffect(() => {
		invoke<boolean>("in_dev_env").then((res) => {
			if (!res) {
				const handleContextMenu = (e: Event) => {
					e.preventDefault();
				};
				document.addEventListener("contextmenu", handleContextMenu);
				return () => {
					document.removeEventListener("contextmenu", handleContextMenu);
				};
			}
		});

		restoreStateCurrent(StateFlags.ALL);
	}, []);

	return (
		<main
			class={`flex h-screen w-screen flex-col gap-0 ${textTheme({ game: game })}`}
		>
			<Titlebar />
			<Sidebar />
			<PhotosensitivityModal />
			<SettingsModal ref={settingsModal} />

			<div class={bgTheme({ game: game })}>
				<div class="relative h-full w-full">
					{graphics ? <Background /> : null}
				</div>
				<section class="absolute inset-0 z-10 flex w-full flex-row items-end justify-end gap-x-3 px-15 py-10">
					{/* Page content */}
					<DownloadProgress />
					<PreinstallButton />
					<Button
						variant="secondary"
						onClick={() => {
							settingsModal.current?.open();
						}}
						width={4}
						height={4}
					>
						<Settings className="-m-1 leading-0" />
					</Button>
					<InstallerButton />
				</section>
			</div>
		</main>
	);
};

export default function AppWrapper() {
	return (
		<GameProvider>
			<ApiProvider>
				<DownloadProvider>
					<BackgroundProvider>
						<App />
					</BackgroundProvider>
				</DownloadProvider>
			</ApiProvider>
		</GameProvider>
	);
}
