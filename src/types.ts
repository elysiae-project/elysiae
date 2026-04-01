export enum Variants {
	BH3,
	HK4E,
	HKRPG,
	NAP,
}

export enum BhServers {
	GLB,
	JP,
	KR,
	SEA,
	TW,
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

export type LauncherBrandingRawGameData = {
	id: string;
	biz: string;
	display: {
		language: string;
		name: string;
		icon: {
			url: string;
			hover_url: string;
			link: string;
			login_state_in_link: boolean;
			md5: string;
			size: number;
		};
		title: string;
		subtitle: string;
		background: {
			url: string;
			link: string;
			login_state_in_link: boolean;
		};
		logo: {
			url: string;
			link: string;
			login_state_in_link: boolean;
		};
		thumbnail: {
			url: string;
			link: string;
			login_state_in_link: boolean;
		};
		korea_rating: any;
		shortcut: {
			url: string;
			hover_url: string;
			link: string;
			login_state_in_link: boolean;
			md5: string;
			size: number;
		};
		wpf_icon?: {
			url: string;
			hover_url: string;
			link: string;
			login_state_in_link: boolean;
			md5: string;
			size: number;
		};
	};
	reservation: any;
	display_status: string;
	game_server_configs: {
		i18n_name: string;
		i18n_description: string;
		package_name: string;
		auto_scan_registry_key: string;
		package_detection_info: string;
		game_id: string;
		reservation: any;
		display_status: string;
	}[];
};

export type LauncherBrandingRawData = {
	retcode: number;
	message: string;
	data: {
		games: LauncherBrandingRawGameData[];
	};
};
export type LauncherBrandingData = {
	[key in Variants]: {
		id: string;
		icon: string;
		iconLarge: string;
	};
};

export type Download = {
	downloaded: number;
	total: number;
};

export type WineComponent = {
	componentName: string;
	extractTo: string;
	saveTo: string;
	postInstall?: () => Promise<void>;
};

export type ComponentData = {
	tag: string;
	download_url: string;
	hash: string;
};

export type WineModule = {
	name: string;
	downloadLink: string;
	moduleType: "exe" | "dll32" | "dll64";
};