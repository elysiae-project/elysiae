import "./App.css";
import { useGame } from "./hooks/useGame.ts";
import Titlebar from "./components/Titlebar.tsx";
import { cva } from "class-variance-authority";
import { Variants } from "./types";
import { useApi } from "./hooks/useApi.ts";
import Sidebar from "./components/Sidebar.tsx";
import { ApiProvider } from "./contexts/ApiContext.tsx";
import { GameContext, GameProvider } from "./contexts/GameContext.tsx";
import Button from "./components/Button.tsx";
import { createWineEnv, wineEnvAvailable } from "./util/WineTools.ts";
import { downloadGame } from "./util/GameManager.ts";
import { useEffect, useState } from "preact/hooks";
import { error, info } from "@tauri-apps/plugin-log";
import { join, resourceDir } from "@tauri-apps/api/path";
import { getActiveGameCode } from "./util/AppFunctions.ts";

const theme = cva("h-full w-full overflow-hidden", {
	variants: {
		intent: {
			[Variants.BH]: "bg-bh-bg font-bh-sr rounded-b-xl text-white",
			[Variants.YS]: "bg-ys-bg font-ys text-black",
			[Variants.SR]:
				"bg-sr-bg font-bh-sr rounded-b-xs border border-[#393939] text-black",
			[Variants.NAP]:
				"bg-nap-bg font-nap rounded-br-xl border-b-2 border-r-2 border-l-2 border-nap-border text-white",
		},
	},
});

function Background() {}

function App() {
	const { game } = useGame();
	const { graphics, gamePackages } = useApi();

	let [wineEnvExists, setWineEnvExists] = useState(false);

	useEffect(() => {
		wineEnvAvailable().then((res) => {
			setWineEnvExists(res);
		});
	}, []);

	return (
		<div class="flex h-screen w-screen flex-col gap-0 text-white">
			<Titlebar />

			<div class={theme({ intent: game })}>
				{graphics && gamePackages ? (
					<div class="relative h-full w-full">
						<video
							class="absolute inset-0 h-full w-full object-cover"
							src={graphics[game].backgroundVideo}
							autoplay
							loop
							muted
						></video>
						<img
							class="absolute inset-0 h-full w-full object-cover"
							src={graphics[game].backgroundVideoOverlay}
							alt=""
						/>

						<div class="absolute inset-0 z-10 flex flex-row items-end justify-end px-15 py-10 w-full">
							{/* Page content */}
							<Button
								intent="primary"
								onClick={async () => {
									if (wineEnvExists) {
										// TODO: Download links processing
										// await downloadGame([], "");
										const downloadPath = await join((await resourceDir()), getActiveGameCode());

										await downloadGame(gamePackages[game].main.major.game_pkgs, "");
										info("The game is being downloaded ooooooooooo oooooooo");
									} else {
										// Download wine instead
										await createWineEnv()
											.then(() => {
												setWineEnvExists(true);
											})
											.catch((e) => {
												error(`Error in creating wine environment: ${e}`);
											});
									}
								}}
							>
								{wineEnvExists ? "Download Game" : "Create Environment"}
							</Button>
						</div>

						<Sidebar />
					</div>
				) : (
					<div class="flex h-full w-full flex-col items-center justify-center text-center">
						<h2 class="text-6xl">Loading...</h2>
					</div>
				)}
			</div>
		</div>
	);
}

export default function AppWrapper() {
	return (
		<GameProvider>
			<ApiProvider>
				<App />
			</ApiProvider>
		</GameProvider>
	);
}
