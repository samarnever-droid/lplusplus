import Nav from "./components/Nav";
import Hero from "./components/Hero";
import Marquee from "./components/Marquee";
import Pillars from "./components/Pillars";
import MemoryModel from "./components/MemoryModel";
import Showcase from "./components/Showcase";
import Performance from "./components/Performance";
import Stdlib from "./components/Stdlib";
import Roadmap from "./components/Roadmap";
import Install from "./components/Install";
import Footer from "./components/Footer";

export default function App() {
  return (
    <div className="noise relative min-h-screen bg-ink font-sans text-white antialiased">
      <Nav />
      <main>
        <Hero />
        <Marquee />
        <Pillars />
        <MemoryModel />
        <Showcase />
        <Performance />
        <Stdlib />
        <Roadmap />
        <Install />
      </main>
      <Footer />
    </div>
  );
}
