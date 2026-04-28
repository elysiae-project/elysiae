import "./App.css";
import Button from "./components/Button.tsx";
import Background from "./components/Background.tsx";
import Sidebar from "./components/app/Sidebar.tsx";
import Titlebar from "./components/Titlebar.tsx";
import { useGame } from "./hooks/useGame.ts";
import { cva } from "class-variance-authority";
import { Variants } from "./types";
import { useApi } from "./hooks/useApi.ts";
import { ApiProvider } from "./contexts/ApiContext.tsx";
import { GameProvider } from "./contexts/GameContext.tsx";
import { Settings } from "lucide-preact";
import DownloadProgress from "./components/app/DownloadProgress.tsx";
import PreinstallButton from "./components/app/PreinstallButton.tsx";
import InstallerButton from "./components/app/InstallerButton.tsx";
import { DownloadProvider } from "./contexts/DownloadContext.tsx";

const theme = cva("h-full w-full overflow-hidden", {
	variants: {
		intent: {
			[Variants.BH3]: "bg-bh3-bg font-bh3-hkrpg rounded-b-xl text-white",
			[Variants.HK4E]: "bg-hk4e-bg font-hk4e text-black",
			[Variants.HKRPG]: "bg-hkrpg-bg font-bh3-hkrpg rounded-b-xs text-black",
			[Variants.NAP]:
				"bg-nap-bg font-nap rounded-br-xl border-nap-border text-white",
		},
	},
});

function App() {
	const { game } = useGame();
	const { graphics } = useApi();
	

	return (
		<div class="flex h-screen w-screen flex-col gap-0 text-white">
			<Titlebar />

			<div class={theme({ intent: game })}>
				<div class="relative h-full w-full">
					{graphics ? <Background /> : <></>}


					<div class="absolute inset-0 z-10 flex flex-row items-end justify-end px-15 py-10 w-full gap-x-3">
						{/* Page content */}
						<DownloadProgress />
						<PreinstallButton />
						<Button
							intent="secondary"
							onClick={() => {
								//setSettingsOpen(true);
							}}
							iconButton>
							<Settings className="leading-0 -m-1" />
						</Button>
						<InstallerButton />
					</div>
					<Sidebar />
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
