import { useState } from "react";
import Nav from "./components/Nav";
import Hero from "./components/Hero";
import Marquee from "./components/Marquee";
import Pillars from "./components/Pillars";
import MemoryModel from "./components/MemoryModel";
import Showcase from "./components/Showcase";
import Performance from "./components/Performance";
import PackagesShowcase from "./components/PackagesShowcase";
import Stdlib from "./components/Stdlib";
import Roadmap from "./components/Roadmap";
import Install from "./components/Install";
import Footer from "./components/Footer";
import RegistryModal from "./components/RegistryModal";
import AuthModal from "./components/AuthModal";

export default function App() {
  const [registryOpen, setRegistryOpen] = useState(false);
  const [authOpen, setAuthOpen] = useState(false);

  return (
    <div className="noise relative min-h-screen bg-ink font-sans text-white antialiased">
      <Nav
        onOpenRegistry={() => setRegistryOpen(true)}
        onOpenAuth={() => setAuthOpen(true)}
      />
      <main>
        <Hero />
        <Marquee />
        <Pillars />
        <MemoryModel />
        <Showcase />
        <Performance />
        <PackagesShowcase />
        <Stdlib />
        <Roadmap />
        <Install />
      </main>
      <Footer />

      <RegistryModal
        isOpen={registryOpen}
        onClose={() => setRegistryOpen(false)}
        onOpenAuth={() => setAuthOpen(true)}
      />

      <AuthModal
        isOpen={authOpen}
        onClose={() => setAuthOpen(false)}
      />
    </div>
  );
}
