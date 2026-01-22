export enum Variants {
	BH,
	YS,
	SR,
	NAP,
}

export type LauncherGraphicsRawGameData = {
	game: {
		id: string;
		biz: string;
	};
	backgrounds: {
		id: string;
		background: {
			url: string;
			link: string;
			login_state_in_link: boolean;
		};
		icon: {
			url: string;
			hover_url: string;
			link: string;
			login_state_in_link: boolean;
			md5: string;
			size: number;
		};
		video: {
			url: string;
			size: number;
		};
		theme: {
			url: string;
			link: string;
			login_state_in_link: boolean;
		};
		type: string;
	}[];
};
export type LauncherGraphicsRawData = {
	retcode: number;
	message: string;
	data: {
		game_info_list: LauncherGraphicsRawGameData[];
	};
};
export type LauncherGraphicsData = {
	[key in Variants]: {
		backgroundImage: string;
		backgroundVideo: string;
		backgroundVideoOverlay: string;
		icon: string;
	};
};
