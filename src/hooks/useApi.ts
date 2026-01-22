import { fetch } from "@tauri-apps/plugin-http";
import { useEffect, useState } from "preact/hooks";
import {
	LauncherGraphicsData,
	LauncherGraphicsRawData,
	LauncherGraphicsRawGameData,
	Variants,
} from "../types";

const LAUNCHER_ID = "VYTpXlbWo8";
const LANGUAGE = "en";

const BH3_EN_ID = "bxPTXSET5t";

let loading = false;
export const useApi = () => {
	const [data, setData] = useState<LauncherGraphicsData | null>(null);

	useEffect(() => {
		if (!data && !loading) {
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
					setData(formatted);
				})
				.catch((err) => console.log(err))
				.finally(() => (loading = false));
		}
	}, []);

	return data;
};
