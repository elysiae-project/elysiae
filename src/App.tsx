import "./App.css";
import { useGame } from "./hooks/useGame.ts";
import Titlebar from "./components/Titlebar.tsx";
import { cva } from "class-variance-authority";
import { Variants } from "./types";
import { useApi } from "./hooks/useApi.ts";
import Sidebar from "./components/Sidebar.tsx";
import { ApiProvider } from "./contexts/ApiContext.tsx";
import { GameProvider } from "./contexts/GameContext.tsx";
import Button from "./components/Button.tsx";
import { useEffect, useState } from "preact/hooks";
import { Save } from "lucide-preact";
import { updateWineComponents, wineEnvAvailable } from "./lib/WineManager.ts";

const theme = cva("h-full w-full overflow-hidden", {
	variants: {
		intent: {
			[Variants.BH3]: "bg-bh-bg font-bh-sr rounded-b-xl text-white",
			[Variants.HK4E]: "bg-ys-bg font-ys text-black",
			[Variants.HKRPG]:
				"bg-sr-bg font-bh-sr rounded-b-xs border border-[#393939] text-black",
			[Variants.NAP]:
				"bg-nap-bg font-nap rounded-br-xl border-b-2 border-r-2 border-l-2 border-nap-border text-white",
		},
	},
});

function PreinstallButton() {
	// A lot of placeholder stuff here. Just want to get the component to render so I can implement this stuff in the future
	let [preInstAvailable, setPreInstAvailable] = useState<boolean>(false); // Not implemented yet
	const { game } = useGame();

	useEffect(() => {
		// TODO: Add preinstall check each time game switches onces sophon downloader is implemented
	}, [game]);

	if (!preInstAvailable) return <></>;

	return (
		<Button intent="primary" overrideMinWidth={true} onClick={async () => {}}>
			<Save />
		</Button>
	);
}

function Background() {
	const { game } = useGame();
	const { graphics } = useApi();
	if (!graphics) return null;

	const url =
		graphics[game].backgroundVideo === ""
			? graphics[game].backgroundImage
			: graphics[game].backgroundVideo;

	return (
		<>
			<img
				class="absolute inset-0 h-full w-full object-cover z-10"
				src={graphics[game].backgroundVideoOverlay}
			/>
			{url.endsWith(".webp") ? (
				<img class="absolute h-full w-full object-cover" src={url} alt="" />
			) : (
				<video
					class="absolute inset-0 h-full w-full object-cover"
					src={url}
					autoplay
					loop
					muted
				/>
			)}
		</>
	);
}

function App() {
	const { game } = useGame();
	const { graphics } = useApi();

	let [wineAvailable, setWineAvailable] = useState<boolean>(false);
	let [gameInstalled, setGameInstalled] = useState<boolean>(false); // TODO: Add game installation checks after the sophon downloader is done

	useEffect(() => {
		wineEnvAvailable().then((res) => {
			setWineAvailable(res);
		});
	}, [game]);

	return (
		<div class="flex h-screen w-screen flex-col gap-0 text-white">
			<Titlebar />

			<div class={theme({ intent: game })}>
				{graphics ? (
					<div class="relative h-full w-full">
						<Background />

						<div class="absolute inset-0 z-10 flex flex-row items-end justify-end px-15 py-10 w-full gap-x-5">
							{/* Page content */}
							<PreinstallButton />
							<Button
								intent="primary"
								onClick={async () => {
									if (!wineAvailable) {
										await updateWineComponents();
									} else if (!gameInstalled) {
										// TODO: Add game downloader functionality
									} else {
										// TODO: Add launch game functionality
									}
								}}
							>
								{!wineAvailable
									? "Create Environment"
									: !gameInstalled
										? "Download"
										: "Launch"}
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
