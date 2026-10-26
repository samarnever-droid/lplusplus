// Extracted from cosmic-canvas-trailer.html: graph, ending, and boot flow. Load in listed order.
      async function sceneGraph() {
        setSceneLabel(4, "Graph");
        setProgress(80);
        caption("Knowledge Graph — traverse related materials");
        url("samarbook.app/graph");

        // Start cursor at bottom center (near dock) while hidden
        state.cursorX = 400;
        state.cursorY = 440;
        cursor.style.left = "400px";
        cursor.style.top = "440px";

        showScene("sc-graph");
        await sleep(250);
        cursorShow(true);

        const svg = $("graph-svg");
        svg.innerHTML = "";

        const nodes = [
          { id: "n1", x: 280, y: 220, label: "Canvas Walkthrough", r: 22, c: "#FF5A36" },
          { id: "n2", x: 480, y: 160, label: "Orbital mechanics", r: 18, c: "#3A86FF" },
          { id: "n3", x: 490, y: 300, label: "Plugin SDK notes", r: 17, c: "#8B5CF6" },
          { id: "n4", x: 290, y: 360, label: "Canvas graph draft", r: 16, c: "#10B981" },
          { id: "n5", x: 140, y: 150, label: "Weekly roadmap", r: 15, c: "#f59e0b" },
          { id: "n6", x: 580, y: 120, label: "Reading list", r: 14, c: "#ec4899" },
          { id: "n7", x: 590, y: 340, label: "Plugin Ideas", r: 13, c: "#f59e0b" },
        ];
        const edges = [
          ["n1", "n2"],
          ["n1", "n3"],
          ["n1", "n4"],
          ["n1", "n5"],
          ["n2", "n6"],
          ["n3", "n7"],
        ];

        // edges first
        for (const [a, b] of edges) {
          const A = nodes.find((n) => n.id === a);
          const B = nodes.find((n) => n.id === b);
          const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
          line.setAttribute("x1", A.x);
          line.setAttribute("y1", A.y);
          line.setAttribute("x2", B.x);
          line.setAttribute("y2", B.y);
          line.setAttribute("stroke", "#334155");
          line.setAttribute("stroke-width", "1.5");
          line.setAttribute("stroke-dasharray", "120");
          line.setAttribute("stroke-dashoffset", "120");
          line.setAttribute("opacity", "0");
          svg.appendChild(line);
        }

        // nodes
        for (const n of nodes) {
          const g = document.createElementNS("http://www.w3.org/2000/svg", "g");
          g.classList.add("g-node");
          g.style.opacity = "0";
          g.style.transformOrigin = `${n.x}px ${n.y}px`;
          g.dataset.id = n.id;

          const circle = document.createElementNS("http://www.w3.org/2000/svg", "circle");
          circle.setAttribute("cx", n.x);
          circle.setAttribute("cy", n.y);
          circle.setAttribute("r", n.r);
          circle.setAttribute("fill", n.c);
          circle.setAttribute("opacity", "0.9");
          circle.setAttribute("filter", `drop-shadow(0 0 8px ${n.c}88)`);

          const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
          text.setAttribute("x", n.x);
          text.setAttribute("y", n.y + n.r + 16);
          text.setAttribute("text-anchor", "middle");
          text.classList.add("g-label");
          text.textContent = n.label;

          g.appendChild(circle);
          g.appendChild(text);
          svg.appendChild(g);
        }

        await sleep(200);

        // Staggered graph line drawing
        anime({
          targets: "#graph-svg line",
          strokeDashoffset: [120, 0],
          opacity: [0, 0.65],
          duration: 1000,
          easing: "easeOutQuart",
          delay: anime.stagger(80),
        });

        // Staggered node grow/pop-in
        anime({
          targets: "#graph-svg g",
          scale: [0, 1],
          opacity: [0, 1],
          duration: 1200,
          easing: "easeOutElastic(1.1, .75)",
          delay: anime.stagger(90),
        });

        await sleep(1500);

        // cursor hover node
        caption("Click node to load connected canvases");
        const n1 = svg.querySelector('[data-id="n1"] circle');
        if (n1) {
          const sr = screen.getBoundingClientRect();
          const nr = n1.getBoundingClientRect();
          await cursorTo(nr.left - sr.left + nr.width / 2, nr.top - sr.top + nr.height / 2, 700);
          click();

          // Pulse bubble elastic zoom
          anime({
            targets: n1,
            r: [22, 28, 22],
            duration: 500,
            easing: "easeOutBack",
          });
          await sleep(600);
        }
        cursorShow(false);
        await sleep(800);
      }

      async function sceneEnd() {
        setSceneLabel(5, "Outro");
        setProgress(100);
        caption("Local-first Knowledge OS");
        url("samarbook.app");
        showScene("sc-end");
        cursorShow(false);

        // Reset Outro layout styles
        anime.set(["#end-h", "#end-p", "#end-cta", "#end-feats span"], {
          opacity: 0,
          translateY: 25,
        });

        const tl = anime.timeline({
          easing: "easeOutCubic",
          duration: 650,
        });

        tl.add({
          targets: "#end-h",
          opacity: [0, 1],
          translateY: [15, 0],
        })
          .add(
            {
              targets: "#end-p",
              opacity: [0, 1],
              translateY: [15, 0],
            },
            "-=450",
          )
          .add(
            {
              targets: "#end-cta",
              opacity: [0, 1],
              scale: [0.92, 1],
              translateY: [15, 0],
              duration: 550,
              easing: "easeOutBack",
            },
            "-=400",
          )
          .add(
            {
              targets: "#end-feats span",
              opacity: [0, 1],
              scale: [0.8, 1],
              translateY: [15, 0],
              delay: anime.stagger(70),
              duration: 750,
              easing: "easeOutBack",
            },
            "-=350",
          );

        await sleep(3500);
      }

      async function loading() {
        const steps = [
          [12, "Booting brand layers…"],
          [28, "Injecting Lucide components…"],
          [45, "Loading audio sound effects…"],
          [65, "Preparing workspace directories…"],
          [82, "Configuring settings and plugin API…"],
          [100, "Ready"],
        ];
        for (const [p, t] of steps) {
          $("loader-fill").style.width = `${p}%`;
          $("loader-text").textContent = t;
          safeCreateIcons();
          await sleep(160 + Math.random() * 80);
        }
        await sleep(250);
        $("loader").classList.add("done");

        // Reveal Play overlay to get user gesture for Web Audio API
        $("play-overlay").classList.remove("hidden");
        safeCreateIcons();
      }

      async function runTrailer() {
        if (state.running) return;
        state.running = true;
        state.abort = false;
        state.paused = false;
        $("btn-pause").textContent = "Pause";
        $("device").classList.add("ready");

        // Stop all active anime.js runs immediately on new run
        if (typeof anime !== "undefined" && anime.running) {
          anime.running.forEach((a) => a.stop());
        }

        state.cursorX = 80;
        state.cursorY = 80;
        cursor.style.left = "80px";
        cursor.style.top = "80px";

        const scenes = [sceneTitle, sceneHome, sceneNotebook, sceneCanvas, sceneGraph, sceneEnd];

        try {
          for (const scene of scenes) {
            if (state.abort) break;
            await scene();
          }
        } catch (err) {
          if (err === ABORT_ERROR) {
            console.log("Trailer execution aborted cleanly.");
          } else {
            console.error("Trailer execution failed:", err);
          }
        } finally {
          state.running = false;
          caption("Trailer finished — click Replay to watch again");
        }
      }

      async function boot() {
        safeCreateIcons();
        await loading();
      }

      $("btn-start-cinematic").addEventListener("click", () => {
        $("play-overlay").classList.add("hidden");
        initAudio();
        toggleMusic();
        runTrailer();
      });

      $("btn-play").addEventListener("click", async () => {
        state.abort = true;
        state.running = false;
        if (typeof anime !== "undefined" && anime.running) {
          anime.running.forEach((a) => a.stop());
        }
        await sleep(100);
        state.abort = false;
        await runTrailer();
      });

      $("btn-pause").addEventListener("click", (e) => {
        state.paused = !state.paused;
        e.target.textContent = state.paused ? "Resume" : "Pause";
      });

      $("btn-skip").addEventListener("click", () => {
        state.paused = false;
        $("btn-pause").textContent = "Pause";
        state.abort = true;
        state.running = false;
        if (typeof anime !== "undefined" && anime.running) {
          anime.running.forEach((a) => a.stop());
        }
        setTimeout(runTrailer, 120);
      });

      $("btn-export-mp4").addEventListener("click", () => {
        playSound("plugin");
        $("dlg-export").classList.add("open");
        runExportSimulation();
      });

      $("btn-cancel-export").addEventListener("click", () => {
        playSound("click");
        if (exportInterval) clearInterval(exportInterval);
        $("dlg-export").classList.remove("open");
      });

      boot();
