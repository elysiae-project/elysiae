import { fetch } from "@tauri-apps/plugin-http";
import { createContext, ComponentChildren } from "preact";
import { useEffect, useState } from "preact/hooks";
import {
	LauncherBrandingData,
	LauncherBrandingRawData,
	LauncherBrandingRawGameData,
	LauncherGraphicsData,
	LauncherGraphicsRawData,
	LauncherGraphicsRawGameData,
	Variants,
} from "../types";

const LAUNCHER_ID = "VYTpXlbWo8";
const LANGUAGE = "en";

const BH3_EN_ID = "bxPTXSET5t";

interface ApiContextType {
	graphics: LauncherGraphicsData | null;
	branding: LauncherBrandingData | null;
}

export const ApiContext = createContext<ApiContextType>({
	graphics: null,
	branding: null,
});

let loading = false;

export const ApiProvider = ({ children }: { children: ComponentChildren }) => {
	const [graphicsData, setGraphicsData] = useState<LauncherGraphicsData | null>(
		null,
	);
	const [brandingData, setBrandingData] = useState<LauncherBrandingData | null>(
		null,
	);

	useEffect(() => {
		if (!loading) {
			loading = true;

			fetch(
				`https://${["sg", "hyp", "api"].join("-")}.hoyoverse.com/${["hyp", "hyp-connect", "api", "getAllGameBasicInfo"].join("/")}?launcher_id=${LAUNCHER_ID}&language=${LANGUAGE}`,
			)
				.then((res) => res.json())
				.then((data: LauncherGraphicsRawData) => {
					if (data.message !== "OK") throw new Error(data.message);
					console.log(data);
					const formatted = data.data.game_info_list.reduce(
						(acc: LauncherGraphicsData, game: LauncherGraphicsRawGameData) => {
							let id;
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
							let id;
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
				branding: brandingData,
			}}>
			{children}
		</ApiContext.Provider>
	);
};
