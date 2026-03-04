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
	LauncherPkgRawData,
	LauncherGamePkgRawData,
	Variants,
	LauncherPkgData,
} from "../types";

const LAUNCHER_ID = "VYTpXlbWo8";
const LANGUAGE = "en";

const BH3_EN_ID = "bxPTXSET5t";

interface ApiContextType {
	graphics: LauncherGraphicsData | null;
	branding: LauncherBrandingData | null;
	gamePackages: LauncherPkgData | null;
}

export const ApiContext = createContext<ApiContextType>({
	graphics: null,
	branding: null,
	gamePackages: null,
});

let loading = false;

export const ApiProvider = ({ children }: { children: ComponentChildren }) => {
	const [graphicsData, setGraphicsData] = useState<LauncherGraphicsData | null>(
		null,
	);
	const [brandingData, setBrandingData] = useState<LauncherBrandingData | null>(
		null,
	);

	const [pkgData, setPkgData] = useState<LauncherPkgData | null>(null);

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
									if (game.game.id === BH3_EN_ID) id = Variants.BH;
									break;
								case "hk4e_global":
									id = Variants.YS;
									break;
								case "hkrpg_global":
									id = Variants.SR;
									break;
								case "nap_global":
									id = Variants.NAP;
									break;
							}
							if (typeof id === "undefined") return acc;
							acc[id] = {
								backgroundImage: game.backgrounds[0].background.url,
								backgroundVideo: game.backgrounds[0].video.url,
								backgroundVideoOverlay: game.backgrounds[0].theme.url,
								icon: game.backgrounds[0].icon.url,
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
									id = Variants.BH;
									break;
								case "hk4e_global":
									id = Variants.YS;
									break;
								case "hkrpg_global":
									id = Variants.SR;
									break;
								case "nap_global":
									id = Variants.NAP;
									break;
							}
							if (typeof id === "undefined") return acc;
							acc[id] = {
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

			fetch(
				`https://${["sg", "hyp", "api"].join("-")}.hoyoverse.com/${["hyp", "hyp-connect", "api", "getGamePackages"].join("/")}?launcher_id=${LAUNCHER_ID}`,
			)
				.then((res) => res.json())
				.then((data: LauncherPkgRawData) => {
					if (data.message !== "OK") throw new Error(data.message);
					console.log(data);

					// TODO: The downloader type json thingy. Too lazy to figure this out right now
					const formatted = data.data.game_packages.reduce(
						(acc: LauncherPkgData, game: LauncherGamePkgRawData) => {
							let id;
							switch (game.game.biz) {
								case "bh3_global":
									id = Variants.BH;
									break;
								case "hk4e_global":
									id = Variants.YS;
									break;
								case "hkrpg_global":
									id = Variants.SR;
									break;
								case "nap_global":
									id = Variants.NAP;
									break;
							}
							if (typeof id === "undefined") return acc;
							// TODO: need to check this logic, may need to handle merging objects/arrays
							acc[id] = game;
							return acc as LauncherPkgData;
						},
						{} as LauncherPkgData,
					);
					console.log(formatted);
					setPkgData(formatted);
				})
				.finally(() => (loading = false));
		}
	}, []);

	return (
		<ApiContext.Provider
			value={{
				graphics: graphicsData,
				branding: brandingData,
				gamePackages: pkgData,
			}}
		>
			{children}
		</ApiContext.Provider>
	);
};
