import "./App.css";
import { Signal, signal } from "@preact/signals";
import { game, variants } from "./util/selectedGame.ts";
import Titlebar from "./Components/Titlebar.tsx";

const selectedGame: Signal<variants> = signal(game);

export default function App() {
  return (
    <main class="h-screen w-screen bg-gray-900 text-white rounded-xl">
        <Titlebar intent={selectedGame.value}/>
        <div class="px-3 py-4">
            <h1>It's Taurin' Time</h1>
        </div>
    </main>
  );
}
