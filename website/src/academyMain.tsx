import React from "react";
import ReactDOM from "react-dom/client";
import Nav from "./components/Nav";
import Academy from "./components/Academy";
import Footer from "./components/Footer";
import "./index.css";

ReactDOM.createRoot(document.getElementById("academy-root")!).render(
  <React.StrictMode>
    <div className="noise relative min-h-screen bg-ink font-sans text-white antialiased">
      <Nav />
      <main className="pt-16">
        <Academy />
      </main>
      <Footer />
    </div>
  </React.StrictMode>
);
