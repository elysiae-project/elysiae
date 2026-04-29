import "./App.css";
import Button from "./components/Button.tsx";
import Background from "./components/Background.tsx";
import Sidebar from "./components/app/Sidebar.tsx";
import Titlebar from "./components/Titlebar.tsx";
import { useGame } from "./hooks/useGame.ts";
import { cva } from "class-variance-authority";
import { ModalHandle, Variants } from "./types";
import { useApi } from "./hooks/useApi.ts";
import { ApiProvider } from "./contexts/ApiContext.tsx";
import { GameProvider } from "./contexts/GameContext.tsx";
import { Settings } from "lucide-preact";
import DownloadProgress from "./components/app/DownloadProgress.tsx";
import PreinstallButton from "./components/app/PreinstallButton.tsx";
import InstallerButton from "./components/app/InstallerButton.tsx";
import { DownloadProvider } from "./contexts/DownloadContext.tsx";
import SettingsModal from "./components/app/SettingsModal.tsx";
import { useRef } from "preact/hooks";

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
			[Variants.HKRPG]: "bg-hkrpg-bg rounded-b-xs ",
			[Variants.NAP]: "bg-nap-bg rounded-br-xl border-nap-border",
		},
	},
});

function App() {
	const { game } = useGame();
	const { graphics } = useApi();
	const settingsModal = useRef<ModalHandle>(null);

	return (
		<div
			class={`flex h-screen w-screen flex-col gap-0 ${textTheme({ game: game })}`}>
			<Titlebar />
			<Sidebar />
			<SettingsModal ref={settingsModal} />

			<div class={bgTheme({ game: game })}>
				<div class="relative h-full w-full">
					{graphics ? <Background /> : <></>}

					<div class="absolute inset-0 z-10 flex flex-row items-end justify-end px-15 py-10 w-full gap-x-3">
						{/* Page content */}
						<DownloadProgress />
						<PreinstallButton />
						<Button
							intent="secondary"
							onClick={() => {
								settingsModal.current?.open();
							}}
							width={65}
							height={65}>
							<Settings className="leading-0 -m-1" />
						</Button>
						<InstallerButton />
					</div>
				</div>
			</div>
		</div>
	);
}

export default function AppWrapper() {
	return (
		<GameProvider>
			<ApiProvider>
				<DownloadProvider>
					<App />
				</DownloadProvider>
			</ApiProvider>
		</GameProvider>
	);
}
