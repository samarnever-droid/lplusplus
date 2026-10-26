// Extracted from cosmic-canvas-trailer.html: shared runtime and interaction helpers. Load in listed order.
      /**
       * On-Screen Uncaught Error Reporter (for diagnostic visual visibility)
       */
      window.addEventListener("error", function (e) {
        const errDiv = document.createElement("div");
        errDiv.style.position = "fixed";
        errDiv.style.top = "0";
        errDiv.style.left = "0";
        errDiv.style.width = "100%";
        errDiv.style.background = "rgba(239, 68, 68, 0.95)";
        errDiv.style.color = "#fff";
        errDiv.style.padding = "12px";
        errDiv.style.zIndex = "99999";
        errDiv.style.fontFamily = "monospace";
        errDiv.style.fontSize = "11px";
        errDiv.style.lineHeight = "1.4";
        errDiv.style.maxHeight = "150px";
        errDiv.style.overflowY = "auto";
        errDiv.style.boxShadow = "0 4px 12px rgba(0,0,0,0.3)";
        errDiv.textContent =
          "Runtime Error: " + e.message + " at " + e.filename + ":" + e.lineno + ":" + e.colno;
        document.body.appendChild(errDiv);
      });

      const $ = (id) => document.getElementById(id);

      const state = {
        running: false,
        paused: false,
        abort: false,
        scene: 0,
        totalScenes: 7,
        constellationActive: false,
        completedEdges: [],
        cursorX: 80,
        cursorY: 80,
      };

      const ABORT_ERROR = new Error("AbortError");

      const sleep = (ms) =>
        new Promise((resolve, reject) => {
          const start = performance.now();
          const tick = (now) => {
            if (state.abort) return reject(ABORT_ERROR);
            if (state.paused) {
              requestAnimationFrame(tick);
              return;
            }
            if (now - start >= ms) resolve();
            else requestAnimationFrame(tick);
          };
          requestAnimationFrame(tick);
        });

      /**
       * Lucide Safe Creator
       */
      function safeCreateIcons() {
        if (typeof lucide !== "undefined" && lucide.createIcons) {
          try {
            lucide.createIcons();
          } catch (e) {
            console.error("Lucide failed to render", e);
          }
        } else {
          console.warn("Lucide library is not loaded yet.");
        }
      }

      /**
       * Web Audio API Sound Effects Engine
       */
      let audioCtx = null;

      function initAudio() {
        if (!audioCtx) {
          try {
            const AudioCtxClass = window.AudioContext || window.webkitAudioContext;
            if (AudioCtxClass) {
              audioCtx = new AudioCtxClass();
            }
          } catch (e) {
            console.warn("AudioContext failed to initialize:", e);
          }
        }
      }

      function playSound(type) {
        try {
          initAudio();
          if (!audioCtx) return;
          if (audioCtx.state === "suspended") {
            audioCtx.resume();
          }

          const now = audioCtx.currentTime;
          const osc = audioCtx.createOscillator();
          const gain = audioCtx.createGain();
          osc.connect(gain);
          gain.connect(audioCtx.destination);

          if (type === "click") {
            osc.type = "sine";
            osc.frequency.setValueAtTime(1000, now);
            osc.frequency.exponentialRampToValueAtTime(100, now + 0.05);
            gain.gain.setValueAtTime(0.15, now);
            gain.gain.exponentialRampToValueAtTime(0.01, now + 0.05);
            osc.start(now);
            osc.stop(now + 0.05);
          } else if (type === "type") {
            osc.type = "triangle";
            osc.frequency.setValueAtTime(1200, now);
            osc.frequency.exponentialRampToValueAtTime(300, now + 0.03);
            gain.gain.setValueAtTime(0.06, now);
            gain.gain.exponentialRampToValueAtTime(0.01, now + 0.03);
            osc.start(now);
            osc.stop(now + 0.03);
          } else if (type === "check") {
            osc.type = "sine";
            osc.frequency.setValueAtTime(523.25, now);
            osc.frequency.setValueAtTime(659.25, now + 0.08);
            gain.gain.setValueAtTime(0.12, now);
            gain.gain.setValueAtTime(0.12, now + 0.08);
            gain.gain.exponentialRampToValueAtTime(0.01, now + 0.25);
            osc.start(now);
            osc.stop(now + 0.25);
          } else if (type === "plugin") {
            osc.type = "sine";
            osc.frequency.setValueAtTime(440, now);
            osc.frequency.exponentialRampToValueAtTime(880, now + 0.15);
            gain.gain.setValueAtTime(0.1, now);
            gain.gain.exponentialRampToValueAtTime(0.01, now + 0.2);
            osc.start(now);
            osc.stop(now + 0.2);
          } else if (type === "success") {
            const osc2 = audioCtx.createOscillator();
            const gain2 = audioCtx.createGain();
            osc2.connect(gain2);
            gain2.connect(audioCtx.destination);

            osc.type = "sine";
            osc.frequency.setValueAtTime(523.25, now); // C5
            osc.frequency.exponentialRampToValueAtTime(1046.5, now + 0.35);
            gain.gain.setValueAtTime(0.08, now);
            gain.gain.exponentialRampToValueAtTime(0.001, now + 0.35);

            osc2.type = "sine";
            osc2.frequency.setValueAtTime(659.25, now + 0.08); // E5
            osc2.frequency.exponentialRampToValueAtTime(1318.5, now + 0.45);
            gain2.gain.setValueAtTime(0.06, now + 0.08);
            gain2.gain.exponentialRampToValueAtTime(0.001, now + 0.45);

            osc.start(now);
            osc.stop(now + 0.35);
            osc2.start(now + 0.08);
            osc2.stop(now + 0.45);
          }
        } catch (e) {
          console.error("Audio engine click error", e);
        }
      }

      /**
       * Background Music controller
       */
      let bgMusic = null;
      try {
        bgMusic = new Audio("https://www.soundhelix.com/examples/mp3/SoundHelix-Song-8.mp3");
        bgMusic.loop = true;
        bgMusic.volume = 0.3;
      } catch (e) {
        console.warn("Background music failed to initialize:", e);
      }
      let musicPlaying = false;

      function toggleMusic() {
        initAudio();
        if (!bgMusic) return;
        const btnSpan = $("btn-music").querySelector("span");
        const btnIcon = $("btn-music").querySelector("[data-lucide], svg, i");
        if (musicPlaying) {
          bgMusic.pause();
          if (btnSpan) btnSpan.textContent = "Muted";
          if (btnIcon) btnIcon.setAttribute("data-lucide", "volume-x");
          musicPlaying = false;
        } else {
          bgMusic.play().catch((e) => console.log("Music play blocked", e));
          if (btnSpan) btnSpan.textContent = "Music On";
          if (btnIcon) btnIcon.setAttribute("data-lucide", "music");
          musicPlaying = true;
        }
        safeCreateIcons();
      }

      $("btn-music").addEventListener("click", toggleMusic);
      $("vol-music").addEventListener("input", (e) => {
        if (bgMusic) bgMusic.volume = e.target.value;
      });

      const cursor = $("cursor");
      const screen = $("screen");

      function setProgress(pct) {
        $("progress").style.width = `${pct}%`;
      }
      function setSceneLabel(n, name) {
        state.scene = n;
        $("scene-num").textContent = `Scene ${n + 1} / ${state.totalScenes} · ${name}`;
      }
      function caption(t) {
        $("cap-text").textContent = t;
        $("caption").classList.add("on");
      }
      function url(path) {
        $("url-bar").textContent = path;
      }
      function toast(msg, ms = 1800) {
        const t = $("toast");
        t.textContent = msg;
        t.classList.add("show");
        setTimeout(() => t.classList.remove("show"), ms);
      }
      function showScene(id) {
        document.querySelectorAll(".scene").forEach((s) => s.classList.remove("active"));
        $(id).classList.add("active");
        safeCreateIcons();
      }

      /* Cursor helpers — coords relative to #screen */
      function cursorShow(v = true) {
        cursor.classList.toggle("show", v);
      }
      function cursorPos() {
        return {
          x: state.cursorX,
          y: state.cursorY,
        };
      }
      function cursorTo(x, y, duration = 650) {
        return new Promise((resolve, reject) => {
          cursorShow(true);
          if (typeof anime !== "undefined") {
            anime.remove(cursor);
          }
          const from = { x: state.cursorX, y: state.cursorY };
          let isFinished = false;
          const anim = anime({
            targets: cursor,
            left: [`${from.x}px`, `${x}px`],
            top: [`${from.y}px`, `${y}px`],
            duration: duration,
            easing: "cubicBezier(.22, 1, .36, 1)",
            complete: () => {
              isFinished = true;
              state.cursorX = x;
              state.cursorY = y;
              resolve();
            },
          });

          const checkPause = () => {
            if (state.abort) {
              isFinished = true;
              anim.stop();
              reject(ABORT_ERROR);
              return;
            }
            if (isFinished) return;
            if (state.paused) {
              anim.pause();
            } else {
              anim.play();
            }
            requestAnimationFrame(checkPause);
          };
          requestAnimationFrame(checkPause);
        });
      }

      function click() {
        const { x, y } = cursorPos();
        const r = document.createElement("div");
        r.className = "ripple";
        r.style.left = `${x + 4}px`;
        r.style.top = `${y + 4}px`;
        screen.appendChild(r);
        cursor.classList.add("clicking");
        playSound("click");
        setTimeout(() => cursor.classList.remove("clicking"), 110);
        setTimeout(() => r.remove(), 550);
      }

      function elCenter(el) {
        if (!el) return { x: 0, y: 0 };
        const sr = screen.getBoundingClientRect();
        const er = el.getBoundingClientRect();
        return {
          x: er.left - sr.left + er.width / 2,
          y: er.top - sr.top + er.height / 2,
        };
      }

      async function clickEl(el, moveMs = 600) {
        if (!el) return;
        const p = elCenter(el);
        await cursorTo(p.x, p.y, moveMs);
        click();
        await sleep(180);
      }

      async function typeInto(inputEl, text, cps = 18) {
        inputEl.classList.add("typing");
        inputEl.value = "";
        for (let i = 0; i < text.length; i++) {
          if (state.abort) throw ABORT_ERROR;
          while (state.paused) await sleep(40);
          inputEl.value = text.slice(0, i + 1);
          playSound("type");
          await sleep(1000 / cps);
        }
        inputEl.classList.remove("typing");
      }

      async function typeMarkdown(node, rawText, renderedHtml, cps = 35) {
        node.innerHTML = "";
        node.style.opacity = "1";
        node.style.fontFamily = '"JetBrains Mono", monospace';
        node.style.whiteSpace = "pre-wrap";
        node.style.fontSize = "11px";

        const caret = document.createElement("span");
        caret.className = "caret";
        node.appendChild(caret);

        for (let i = 0; i < rawText.length; i++) {
          if (state.abort) throw ABORT_ERROR;
          while (state.paused) await sleep(40);
          node.textContent = rawText.slice(0, i + 1);
          node.appendChild(caret);
          playSound("type");
          await sleep(1000 / cps + ("\n#".includes(rawText[i]) ? 100 : 0));
        }
        caret.remove();
        await sleep(600);

        // Transition to rendered HTML
        node.style.transition = "opacity 0.25s";
        node.style.opacity = "0";
        await sleep(250);

        node.innerHTML = renderedHtml;
        node.style.fontFamily = "inherit";
        node.style.whiteSpace = "normal";
        node.style.opacity = "1";
        playSound("click");
        safeCreateIcons();
      }

      function typeIntoTextNode(el, text, cps = 15) {
        return new Promise((resolve, reject) => {
          el.textContent = "";
          let i = 0;
          const frame = () => {
            if (state.abort) return reject(ABORT_ERROR);
            if (state.paused) {
              setTimeout(frame, 50);
              return;
            }
            el.textContent = text.slice(0, i + 1);
            playSound("type");
            i++;
            if (i < text.length) {
              setTimeout(frame, 1000 / cps);
            } else {
              resolve();
            }
          };
          frame();
        });
      }

      /* Real-time Bezier curve connector drawer */
      function drawEdges() {
        const edgesEl = $("edges");
        edgesEl.innerHTML = "";

        const links = state.constellationActive
          ? [
              { from: "card1", to: "card2", type: "curve" },
              { from: "card2", to: "card4", type: "curve" },
              { from: "card4", to: "card3", type: "curve" },
              { from: "card3", to: "card1", type: "curve" },
            ]
          : state.completedEdges;

        for (const edge of links) {
          const elA = $(edge.from);
          const elB = $(edge.to);
          if (!elA || !elB || elA.style.display === "none" || elB.style.display === "none")
            continue;

          const wA = elA.offsetWidth || 200;
          const hA = elA.offsetHeight || 88;
          const wB = elB.offsetWidth || 200;
          const hB = elB.offsetHeight || 88;

          const p1 = {
            x: (parseFloat(elA.style.left) || 0) + wA,
            y: (parseFloat(elA.style.top) || 0) + hA / 2,
          };
          const p2 = {
            x: parseFloat(elB.style.left) || 0,
            y: (parseFloat(elB.style.top) || 0) + hB / 2,
          };

          const mx = (p1.x + p2.x) / 2,
            my = (p1.y + p2.y) / 2 - 22;
          const path = document.createElementNS("http://www.w3.org/2000/svg", "path");

          if (edge.type === "straight") {
            path.setAttribute("d", `M ${p1.x} ${p1.y} L ${p2.x} ${p2.y}`);
          } else {
            path.setAttribute("d", `M ${p1.x} ${p1.y} Q ${mx} ${my} ${p2.x} ${p2.y}`);
          }

          path.setAttribute("stroke", "var(--blue)");
          path.setAttribute("stroke-width", "1.6");
          path.setAttribute("fill", "none");
          path.setAttribute("stroke-opacity", "0.75");
          path.classList.add("permanent");
          path.classList.add("drawn");
          edgesEl.appendChild(path);

          if (edge.type === "label" && edge.label) {
            const midX = (p1.x + p2.x) / 2;
            const midY = (p1.y + p2.y) / 2 - 11;

            const labelGroup = document.createElementNS("http://www.w3.org/2000/svg", "g");
            const rect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
            rect.setAttribute("fill", "#2563eb");
            rect.setAttribute("rx", "4");
            rect.setAttribute("ry", "4");

            const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
            text.setAttribute("fill", "#fff");
            text.setAttribute("font-size", "9px");
            text.setAttribute("font-family", "inherit");
            text.setAttribute("font-weight", "600");
            text.setAttribute("text-anchor", "middle");
            text.setAttribute("dominant-baseline", "central");
            text.textContent = edge.label;

            labelGroup.appendChild(rect);
            labelGroup.appendChild(text);
            edgesEl.appendChild(labelGroup);

            const textLength = edge.label.length * 5.2 + 8;
            rect.setAttribute("x", midX - textLength / 2);
            rect.setAttribute("y", midY - 7);
            rect.setAttribute("width", textLength);
            rect.setAttribute("height", 14);

            text.setAttribute("x", midX);
            text.setAttribute("y", midY);
          }
        }
      }

      /* Card Drag and Drop Animation */
      function dragElTo(elId, targetLeft, targetTop, duration = 1000) {
        return new Promise((resolve) => {
          const el = $(elId);
          const startLeft = parseFloat(el.style.left) || 0;
          const startTop = parseFloat(el.style.top) || 0;

          // Compute cursor coordinates relative to card header
          const sr = screen.getBoundingClientRect();
          const er = el.getBoundingClientRect();
          const cursorStartX = er.left - sr.left + er.width / 2;
          const cursorStartY = er.top - sr.top + 14;

          const targetCursorX = targetLeft + 100;
          const targetCursorY = targetTop + 14;

          cursorTo(cursorStartX, cursorStartY, 350).then(() => {
            if (typeof anime !== "undefined") {
              anime.remove(cursor);
            }
            click();
            cursor.classList.add("grabbing");

            const start = performance.now();
            const ease = (t) => t * (2 - t); // Ease out

            const frame = (now) => {
              if (state.abort) {
                cursor.classList.remove("grabbing");
                return reject(ABORT_ERROR);
              }
              if (state.paused) {
                requestAnimationFrame(frame);
                return;
              }

              const t = Math.min(1, (now - start) / duration);
              const e = ease(t);

              const currentLeft = startLeft + (targetLeft - startLeft) * e;
              const currentTop = startTop + (targetTop - startTop) * e;

              el.style.left = `${currentLeft}px`;
              el.style.top = `${currentTop}px`;

              cursor.style.left = `${cursorStartX + (targetCursorX - cursorStartX) * e}px`;
              cursor.style.top = `${cursorStartY + (targetCursorY - cursorStartY) * e}px`;

              drawEdges();

              if (t < 1) {
                requestAnimationFrame(frame);
              } else {
                cursor.classList.remove("grabbing");
                click();
                resolve();
              }
            };
            requestAnimationFrame(frame);
          });
        });
      }

      /* Card Resize Corner Dragging Animation (WYSIWYG layout adaptive resizing) */
      function resizeElTo(elId, targetWidth, targetHeight, duration = 1000) {
        return new Promise((resolve) => {
          const el = $(elId);
          const startWidth = el.offsetWidth || 200;
          const startHeight = el.offsetHeight || 88;

          const sr = screen.getBoundingClientRect();
          const er = el.getBoundingClientRect();

          // Bottom-right corner coordinates (drag handle)
          const cursorStartX = er.right - sr.left - 6;
          const cursorStartY = er.bottom - sr.top - 6;

          const targetCursorX = er.left - sr.left + targetWidth - 6;
          const targetCursorY = er.top - sr.top + targetHeight - 6;

          cursorTo(cursorStartX, cursorStartY, 350).then(() => {
            if (typeof anime !== "undefined") {
              anime.remove(cursor);
            }
            click();
            cursor.style.cursor = "se-resize";

            const start = performance.now();
            const ease = (t) => t * (2 - t); // Ease out

            const frame = (now) => {
              if (state.abort) {
                cursor.style.cursor = "default";
                return reject(ABORT_ERROR);
              }
              if (state.paused) {
                requestAnimationFrame(frame);
                return;
              }

              const t = Math.min(1, (now - start) / duration);
              const e = ease(t);

              const currentWidth = startWidth + (targetWidth - startWidth) * e;
              const currentHeight = startHeight + (targetHeight - startHeight) * e;

              el.style.width = `${currentWidth}px`;
              el.style.minHeight = `${currentHeight}px`;
              el.style.height = `${currentHeight}px`;

              cursor.style.left = `${cursorStartX + (targetCursorX - cursorStartX) * e}px`;
              cursor.style.top = `${cursorStartY + (targetCursorY - cursorStartY) * e}px`;

              drawEdges();

              if (t < 1) {
                requestAnimationFrame(frame);
              } else {
                cursor.style.cursor = "default";
                click();
                resolve();
              }
            };
            requestAnimationFrame(frame);
          });
        });
      }

      /* Manual edge connection drag-and-drop animation */
      function connectCards(
        fromId,
        toId,
        type = "curve",
        labelText = "",
        fromPort = "right",
        toPort = "left",
        duration = 1100,
      ) {
        return new Promise((resolve, reject) => {
          const elA = $(fromId);
          const elB = $(toId);
          const edgesEl = $("edges");

          const sr = screen.getBoundingClientRect();
          const rA = elA.getBoundingClientRect();
          const rB = elB.getBoundingClientRect();

          // Port coords relative to screen viewport (for cursor movement)
          const portStartX = (fromPort === "left" ? rA.left : rA.right) - sr.left;
          const portStartY = rA.top - sr.top + rA.height / 2;

          const portEndX = (toPort === "left" ? rB.left : rB.right) - sr.left;
          const portEndY = rB.top - sr.top + rB.height / 2;

          cursorTo(portStartX, portStartY, 450).then(() => {
            if (typeof anime !== "undefined") {
              anime.remove(cursor);
            }
            click();
            cursor.classList.add("grabbing");

            // Show key press state indicators if needed
            if (type === "straight") {
              $("key-hud").classList.add("show");
              $("hud-alt").classList.add("pressed");
              playSound("click");
            } else if (type === "label") {
              $("key-hud").classList.add("show");
              $("hud-ctrl").classList.add("pressed");
              playSound("click");
            }

            // Temporary dashed connector line
            const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
            path.setAttribute("stroke", "var(--blue)");
            path.setAttribute("stroke-width", "1.8");
            path.setAttribute("fill", "none");
            path.setAttribute("stroke-dasharray", "4 3");
            path.setAttribute("stroke-opacity", "0.85");
            edgesEl.appendChild(path);

            // Temporary label element if needed
            let tempLabelGroup = null;
            let tempRect = null;
            let tempText = null;

            if (type === "label") {
              tempLabelGroup = document.createElementNS("http://www.w3.org/2000/svg", "g");
              tempRect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
              tempRect.setAttribute("fill", "#2563eb");
              tempRect.setAttribute("rx", "4");
              tempRect.setAttribute("ry", "4");
              tempText = document.createElementNS("http://www.w3.org/2000/svg", "text");
              tempText.setAttribute("fill", "#fff");
              tempText.setAttribute("font-size", "9px");
              tempText.setAttribute("font-family", "inherit");
              tempText.setAttribute("font-weight", "600");
              tempText.setAttribute("text-anchor", "middle");
              tempText.setAttribute("dominant-baseline", "central");
              tempText.textContent = labelText;
              tempLabelGroup.appendChild(tempRect);
              tempLabelGroup.appendChild(tempText);
              edgesEl.appendChild(tempLabelGroup);
            }

            const start = performance.now();
            const ease = (t) => t * (2 - t);

            const frame = (now) => {
              if (state.abort) {
                path.remove();
                if (tempLabelGroup) tempLabelGroup.remove();
                cursor.classList.remove("grabbing");
                $("hud-alt").classList.remove("pressed");
                $("hud-ctrl").classList.remove("pressed");
                $("key-hud").classList.remove("show");
                return reject(ABORT_ERROR);
              }
              if (state.paused) {
                requestAnimationFrame(frame);
                return;
              }

              const t = Math.min(1, (now - start) / duration);
              const e = ease(t);

              const curX = portStartX + (portEndX - portStartX) * e;
              const curY = portStartY + (portEndY - portStartY) * e;

              cursor.style.left = `${curX}px`;
              cursor.style.top = `${curY}px`;

              // Map viewport coords to local coordinates inside scaled #world SVG
              const worldRect = $("world").getBoundingClientRect();
              const scale = worldRect.width / $("world").offsetWidth;

              const localStartX =
                ((fromPort === "left" ? rA.left : rA.right) - worldRect.left) / scale;
              const localStartY = (rA.top - worldRect.top + rA.height / 2) / scale;

              const localCurX = (curX + sr.left - worldRect.left) / scale;
              const localCurY = (curY + sr.top - worldRect.top) / scale;

              let mx = (localStartX + localCurX) / 2;
              let my = (localStartY + localCurY) / 2 - 20;

              if (type === "straight") {
                path.setAttribute(
                  "d",
                  `M ${localStartX} ${localStartY} L ${localCurX} ${localCurY}`,
                );
              } else {
                path.setAttribute(
                  "d",
                  `M ${localStartX} ${localStartY} Q ${mx} ${my} ${localCurX} ${localCurY}`,
                );
              }

              if (type === "label") {
                const midX = (localStartX + localCurX) / 2;
                const midY = (localStartY + localCurY) / 2 - 11;
                const textLength = labelText.length * 5.2 + 8;
                tempRect.setAttribute("x", midX - textLength / 2);
                tempRect.setAttribute("y", midY - 7);
                tempRect.setAttribute("width", textLength);
                tempRect.setAttribute("height", 14);
                tempText.setAttribute("x", midX);
                tempText.setAttribute("y", midY);
              }

              if (t < 1) {
                requestAnimationFrame(frame);
              } else {
                cursor.classList.remove("grabbing");
                click();
                playSound("success");

                path.remove();
                if (tempLabelGroup) tempLabelGroup.remove();

                $("hud-alt").classList.remove("pressed");
                $("hud-ctrl").classList.remove("pressed");
                $("key-hud").classList.remove("show");

                state.completedEdges.push({
                  from: fromId,
                  to: toId,
                  type: type,
                  label: labelText,
                });
                drawEdges();

                state.cursorX = portEndX;
                state.cursorY = portEndY;
                resolve();
              }
            };
            requestAnimationFrame(frame);
          });
        });
      }

      /* Simulation for exporting cinematic trailer to MP4 */
      let exportInterval = null;
      function runExportSimulation() {
        const bar = $("export-bar");
        const pct = $("export-percentage");
        const status = $("export-status");

        let progress = 0;
        bar.style.width = "0%";
        pct.textContent = "0%";

        const steps = [
          { p: 15, text: "Launching canvas recorder thread (60 FPS)..." },
          { p: 35, text: "Capturing viewport DOM frames & animations..." },
          { p: 55, text: "Mixing synth audio tracks and chimes..." },
          { p: 75, text: "Assembling H.264 video container..." },
          { p: 90, text: "Multiplexing video and audio streams..." },
          { p: 100, text: "Export complete! Downloading MP4 file..." },
        ];

        if (exportInterval) clearInterval(exportInterval);

        exportInterval = setInterval(() => {
          progress += 1.5 + Math.random() * 2.5;
          if (progress >= 100) {
            progress = 100;
            clearInterval(exportInterval);

            setTimeout(() => {
              try {
                const blob = new Blob(["SamarBook Cosmic Canvas Trailer Export"], {
                  type: "video/mp4",
                });
                const url = URL.createObjectURL(blob);
                const a = document.createElement("a");
                a.href = url;
                a.download = "cosmic-canvas-trailer.mp4";
                document.body.appendChild(a);
                a.click();
                document.body.removeChild(a);
                URL.revokeObjectURL(url);
                toast("🎥 Trailer exported successfully!");
              } catch (e) {
                console.error(e);
              }
              $("dlg-export").classList.remove("open");
              playSound("success");
            }, 800);
          }

          bar.style.width = `${progress}%`;
          pct.textContent = `${Math.floor(progress)}%`;

          const currentStep = steps.find((s) => progress <= s.p) || steps[steps.length - 1];
          status.textContent = currentStep.text;

          if (Math.floor(progress) % 15 === 0) {
            playSound("type");
          }
        }, 150);
      }

      /* Settings Panel Tab Swapping */
      function showSettingsTab(tabId) {
        playSound("click");
        document
          .querySelectorAll(".settings-nav-btn")
          .forEach((btn) => btn.classList.remove("active"));
        document
          .querySelectorAll(".settings-pane")
          .forEach((pane) => (pane.style.display = "none"));

        $(`tab-btn-${tabId}`).classList.add("active");
        $(`pane-${tabId}`).style.display = "block";
        $("settings-title").textContent = tabId.charAt(0).toUpperCase() + tabId.slice(1);
        safeCreateIcons();
      }

      $("tab-btn-general").addEventListener("click", () => showSettingsTab("general"));
      $("tab-btn-share").addEventListener("click", () => showSettingsTab("share"));
      $("tab-btn-plugins").addEventListener("click", () => showSettingsTab("plugins"));
      $("btn-close-settings").addEventListener("click", () => {
        playSound("click");
        $("dlg-settings").classList.remove("open");
      });

      /* Constellation Arrangement Animation using Anime.js spring easing */
      function animateCardsToCircle(cx, cy, radius, duration = 1200) {
        return new Promise((resolve) => {
          const cards = ["card1", "card2", "card3", "card4"];
          const anims = [];

          cards.forEach((c, i) => {
            const el = $(c);
            const startLeft = parseFloat(el.style.left) || 0;
            const startTop = parseFloat(el.style.top) || 0;

            const angle = (Math.PI * 2 * i) / cards.length;
            const targetLeft = cx + Math.cos(angle) * radius - 100;
            const targetTop = cy + Math.sin(angle) * radius - 44;

            const a = anime({
              targets: el,
              left: `${targetLeft}px`,
              top: `${targetTop}px`,
              duration: duration,
              easing: "easeOutElastic(1.1, .78)", // Spring bounce effect!
              changeComplete: () => {
                if (i === cards.length - 1) resolve();
              },
              update: () => {
                if (state.abort) a.stop();
                drawEdges();
              },
            });
            anims.push(a);
          });

          const checkPause = () => {
            if (state.abort) {
              anims.forEach((a) => a.stop());
              return;
            }
            anims.forEach((a) => {
              if (state.paused) a.pause();
              else a.play();
            });
            if (anims.every((a) => a.completed)) return;
            requestAnimationFrame(checkPause);
          };
          requestAnimationFrame(checkPause);
        });
      }

