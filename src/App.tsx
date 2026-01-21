import "./App.css";
import {GameContext, useGame} from "./util/selectedGame.ts";
import Titlebar from "./Components/Titlebar.tsx";


export default function App() {
  return (
      <GameContext.Provider value={useGame()}>
          <main class="h-screen w-screen bg-gray-900 text-white rounded-xl">
              <Titlebar/>
              <div class="px-3 py-4">
                  <h1>It's Taurin' Time</h1>
              </div>
          </main>
      </GameContext.Provider>
  );
}
