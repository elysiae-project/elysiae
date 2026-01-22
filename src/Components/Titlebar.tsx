import { getCurrentWindow, Window } from '@tauri-apps/api/window';

import BhTitlebar from './bh/Titlebar.tsx';
import YsTitlebar from './ys/Titlebar.tsx';
import SrTitlebar from './sr/Titlebar.tsx';
import NapTitlebar from './nap/Titlebar.tsx';
import { useGame } from '../util/selectedGame.ts';

const appWindow: Window = getCurrentWindow();

const closeWindow = () => {
	appWindow.close();
};

const toggleMaximize = () => {
	appWindow.toggleMaximize();
};

const minimize = () => {
	appWindow.minimize();
};

export default function Titlebar() {
  const game = useGame();
	switch (game) {
		case 'bh':
			return <BhTitlebar />;
		case 'ys':
			return <YsTitlebar />;
		case 'sr':
			return <SrTitlebar />;
		case 'nap':
			return <NapTitlebar />;
	}
}
