import { fetch } from "@tauri-apps/plugin-http";
import { type ComponentChildren, createContext } from "preact";
import { useEffect, useState } from "preact/hooks";
import {
	type GameCodes,
	type LauncherBackgroundData,
	type LauncherBackgroundRawData,
	type LauncherBrandingData,
	type LauncherBrandingRawData,
	type LauncherBrandingRawGameData,
	type LauncherGraphicsData,
	type LauncherGraphicsRawData,
	type LauncherGraphicsRawGameData,
	Variants,
} from "../types";

const GITHUB_ASSET_BASE =
	"https://raw.githubusercontent.com/elysiae-project/assets/refs/heads/main";
const LAUNCHER_ID = "VYTpXlbWo8";
const LANGUAGE = "en";

const BH3_EN_ID = "bxPTXSET5t";

const GAME_CODE_TO_VARIANT: Record<GameCodes, Variants> = {
	nap: Variants.NAP,
	hkrpg: Variants.HKRPG,
	hk4e: Variants.HK4E,
	bh3: Variants.BH3,
};

interface ApiContextType {
	graphics: LauncherGraphicsData | null;
	backgrounds: LauncherBackgroundData | null;
	branding: LauncherBrandingData | null;
}

export const ApiContext = createContext<ApiContextType>({
	graphics: null,
	backgrounds: null,
	branding: null,
});

let loading = false;

export const ApiProvider = ({ children }: { children: ComponentChildren }) => {
	const [graphicsData, setGraphicsData] = useState<LauncherGraphicsData | null>(
		null,
	);
	const [backgroundData, setBackgroundData] =
		useState<LauncherBackgroundData | null>(null);
	const [brandingData, setBrandingData] = useState<LauncherBrandingData | null>(
		null,
	);

	useEffect(() => {
		if (!loading) {
			loading = true;

			fetch(
				"https://raw.githubusercontent.com/elysiae-project/assets/refs/heads/main/launcherAssets.json",
			).then((res) => {
				res.json().then((data: LauncherBackgroundRawData) => {
					const formatted = (Object.keys(data) as GameCodes[]).reduce(
						(acc: LauncherBackgroundData, code: GameCodes) => {
							const variant = GAME_CODE_TO_VARIANT[code];
							const entry = data[code];

							const bg = entry.backgrounds.find((b) => b.video !== null) ??
								entry.backgrounds[0];

							acc[variant] = {
								backgroundImage: `${GITHUB_ASSET_BASE}/${bg.image}`,
								backgroundVideo: `${GITHUB_ASSET_BASE}/${bg.video}`,
							};
							return acc;
						},
						{} as LauncherBackgroundData,
					);
					setBackgroundData(formatted);
				});
			});

			fetch(
				`https://${["sg", "hyp", "api"].join("-")}.hoyoverse.com/${["hyp", "hyp-connect", "api", "getAllGameBasicInfo"].join("/")}?launcher_id=${LAUNCHER_ID}&language=${LANGUAGE}`,
			)
				.then((res) => res.json())
				.then((data: LauncherGraphicsRawData) => {
					if (data.message !== "OK") throw new Error(data.message);
					console.log(data);
					const formatted = data.data.game_info_list.reduce(
						(acc: LauncherGraphicsData, game: LauncherGraphicsRawGameData) => {
							let id: Variants | undefined;
							switch (game.game.biz) {
								case "bh3_global":
									if (game.game.id === BH3_EN_ID) id = Variants.BH3;
									break;
								case "hk4e_global":
									id = Variants.HK4E;
									break;
								case "hkrpg_global":
									id = Variants.HKRPG;
									break;
								case "nap_global":
									id = Variants.NAP;
									break;
							}
							if (typeof id === "undefined" || game.backgrounds.length === 0)
								return acc;

							const bg =
								game.backgrounds.find(
									(b) => b.type === "BACKGROUND_TYPE_VIDEO",
								) ?? game.backgrounds[0];
							if (!bg) return acc;

							acc[id] = {
								backgroundImage: bg.background.url,
								backgroundVideo: bg.video.url,
								backgroundVideoOverlay: bg.theme.url,
								icon: bg.icon.url,
							};
							return acc;
						},
						{} as LauncherGraphicsData,
					);
					console.log(formatted);
					setGraphicsData(formatted);
				})
				.catch((err) => console.log(err))
				.finally(() => (loading = false));

			fetch(
				`https://${["sg", "hyp", "api"].join("-")}.hoyoverse.com/${["hyp", "hyp-connect", "api", "getGames"].join("/")}?launcher_id=${LAUNCHER_ID}`,
			)
				.then((res) => res.json())
				.then((data: LauncherBrandingRawData) => {
					if (data.message !== "OK") throw new Error(data.message);
					console.log(data);
					const formatted = data.data.games.reduce(
						(acc: LauncherBrandingData, game: LauncherBrandingRawGameData) => {
							let id: Variants | undefined;
							switch (game.biz) {
								case "bh3_global":
									id = Variants.BH3;
									break;
								case "hk4e_global":
									id = Variants.HK4E;
									break;
								case "hkrpg_global":
									id = Variants.HKRPG;
									break;
								case "nap_global":
									id = Variants.NAP;
									break;
							}
							if (typeof id === "undefined") return acc;
							acc[id] = {
								id: game.id,
								icon: game.display.icon.url,
								iconLarge: game.display.shortcut.url,
							};
							return acc;
						},
						{} as LauncherBrandingData,
					);
					console.log(formatted);
					setBrandingData(formatted);
				})
				.catch((err) => console.log(err))
				.finally(() => (loading = false));
		}
	}, []);

	return (
		<ApiContext.Provider
			value={{
				graphics: graphicsData,
				backgrounds: backgroundData,
				branding: brandingData,
			}}
		>
			{children}
		</ApiContext.Provider>
	);
};
