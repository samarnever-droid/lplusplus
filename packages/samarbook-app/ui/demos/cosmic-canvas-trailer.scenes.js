// Extracted from cosmic-canvas-trailer.html: trailer scene implementations. Load in listed order.
      /* ─── SCENES ─── */

      /* Staggered card entrance reveal animation */
      function revealCard(cardId) {
        const el = $(cardId);
        el.style.display = "block";
        el.style.opacity = "0";
        el.style.transform = "scale(0.85) translateY(15px)";
        anime({
          targets: el,
          opacity: [0, 1],
          scale: [0.85, 1],
          translateY: [15, 0],
          duration: 550,
          easing: "easeOutBack",
        });
        el.classList.add("show");
      }

      async function sceneTitle() {
        setSceneLabel(0, "Title");
        setProgress(5);
        caption("SamarBook — Cosmic Canvas");
        url("samarbook.app");
        showScene("sc-title");
        cursorShow(false);

        // Reset layout elements
        anime.set(["#title-logo", "#title-h", "#title-p", "#title-tags span"], {
          opacity: 0,
          translateY: 35,
          scale: 0.95,
        });

        const tl = anime.timeline({
          easing: "easeOutQuart",
          duration: 900,
        });

        tl.add({
          targets: "#title-logo",
          opacity: [0, 1],
          scale: [0.72, 1],
          translateY: [35, 0],
          duration: 800,
        })
          .add(
            {
              targets: "#title-h",
              opacity: [0, 1],
              translateY: [25, 0],
              duration: 800,
            },
            "-=600",
          )
          .add(
            {
              targets: "#title-p",
              opacity: [0, 1],
              translateY: [20, 0],
              duration: 800,
            },
            "-=650",
          )
          .add(
            {
              targets: "#title-tags span",
              opacity: [0, 1],
              translateY: [15, 0],
              scale: [0.9, 1],
              delay: anime.stagger(100),
              duration: 650,
            },
            "-=600",
          );

        await sleep(2600);
      }

      async function sceneHome() {
        setSceneLabel(1, "Homepage");
        setProgress(15);
        caption("Workspace Home — notebooks, import and templates");
        url("samarbook.app/app");

        // Set starting position of cursor for stability
        state.cursorX = 80;
        state.cursorY = 80;
        cursor.style.left = "80px";
        cursor.style.top = "80px";

        showScene("sc-home");
        cursorShow(true);

        const h = new Date().getHours();
        const g = h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
        $("greet").textContent = `${g} 👋`;

        // reset notebooks to cold start empty state
        $("nb-physics").style.display = "none";
        $("nb-project").style.display = "none";
        $("nb-new").style.display = "none";
        $("nb-new").classList.remove("show", "hot");
        $("panel-recent").style.display = "none";
        $("panel-pinned").style.display = "none";
        $("dlg-nb").classList.remove("open");
        $("dlg-nb-input").value = "";
        $("qa-nb").classList.remove("hot");

        // Staggered dashboard grid fade-in (header & quick actions only)
        anime.set(["#greet", "h2", ".qa-grid .qa"], {
          opacity: 0,
          translateY: 20,
        });

        const tl = anime.timeline({
          easing: "easeOutCubic",
          duration: 600,
        });

        tl.add({
          targets: "#greet",
          opacity: [0, 1],
          translateY: [15, 0],
        })
          .add(
            {
              targets: "h2",
              opacity: [0, 1],
              translateY: [15, 0],
            },
            "-=400",
          )
          .add(
            {
              targets: ".qa-grid .qa",
              opacity: [0, 1],
              translateY: [20, 0],
              delay: anime.stagger(80),
            },
            "-=350",
          );

        await sleep(1500);

        // Cursor → New Notebook
        caption("Create a new notebook workspace");
        await clickEl($("qa-nb"), 700);
        $("qa-nb").classList.add("hot");
        await sleep(200);

        // Zoom in notebook dialog
        anime.set("#dlg-nb .modal", { scale: 0.9, opacity: 0 });
        $("dlg-nb").classList.add("open");
        anime({
          targets: "#dlg-nb .modal",
          scale: [0.9, 1],
          opacity: [0, 1],
          duration: 350,
          easing: "easeOutBack",
        });
        await sleep(350);

        caption("Name the notebook");
        await clickEl($("dlg-nb-input"), 400);
        await typeInto($("dlg-nb-input"), "Cosmic Ideas", 16);
        await sleep(250);

        caption("Confirm notebook creation");
        await clickEl($("dlg-nb-ok"), 450);

        anime({
          targets: "#dlg-nb .modal",
          scale: 0.9,
          opacity: 0,
          duration: 200,
          easing: "easeInQuad",
          complete: () => $("dlg-nb").classList.remove("open"),
        });
        $("qa-nb").classList.remove("hot");
        toast("Notebook created");

        // Dynamically insert notebook into grid
        $("nb-new").style.display = "block";
        anime({
          targets: "#nb-new",
          opacity: [0, 1],
          scale: [0.6, 1],
          duration: 500,
          easing: "easeOutBack",
        });
        $("nb-new").classList.add("show", "hot");
        await sleep(1350);

        caption("Open the new notebook");
        await clickEl($("nb-new"), 550);
        cursorShow(false);
        await sleep(300);
      }

      async function sceneNotebook() {
        setSceneLabel(2, "Notebook");
        setProgress(30);
        caption("Notebook dashboard — manage folders & files");
        url("samarbook.app/app · Cosmic Ideas");

        // Set starting position of cursor near new folder button while hidden
        state.cursorX = 180;
        state.cursorY = 180;
        cursor.style.left = "180px";
        cursor.style.top = "180px";

        showScene("sc-notebook");
        await sleep(250);
        cursorShow(true);

        // reset
        $("folder-research").style.display = "none";
        $("folder-research").classList.remove("show");
        $("folder-notes").classList.add("show");
        $("notes-empty").style.display = "block";
        $("note-row").style.display = "none";
        $("note-row").classList.remove("show", "hot");
        $("dlg-note").classList.remove("open");
        $("dlg-folder").classList.remove("open");
        $("dlg-note-input").value = "";
        $("dlg-folder-input").value = "";
        $("btn-new-note").classList.remove("hot");
        $("btn-new-folder").classList.remove("hot");
        $("nb-stats").textContent = "0 notes · 1 folder";
        $("folder-count").textContent = "0";

        await sleep(400);

        // New folder
        caption("Create a structured folder");
        await clickEl($("btn-new-folder"), 600);
        $("btn-new-folder").classList.add("hot");

        anime.set("#dlg-folder .modal", { scale: 0.9, opacity: 0 });
        $("dlg-folder").classList.add("open");
        anime({
          targets: "#dlg-folder .modal",
          scale: [0.9, 1],
          opacity: [0, 1],
          duration: 350,
          easing: "easeOutBack",
        });
        await sleep(300);

        await clickEl($("dlg-folder-input"), 350);
        await typeInto($("dlg-folder-input"), "Research", 18);
        await sleep(200);

        const folderOk = $("dlg-folder").querySelector(".btn-ok");
        await clickEl(folderOk, 400);

        anime({
          targets: "#dlg-folder .modal",
          scale: 0.9,
          opacity: 0,
          duration: 200,
          easing: "easeInQuad",
          complete: () => $("dlg-folder").classList.remove("open"),
        });
        $("btn-new-folder").classList.remove("hot");

        $("folder-research").style.display = "flex";
        anime({
          targets: "#folder-research",
          opacity: [0, 1],
          scale: [0.85, 1],
          duration: 400,
          easing: "easeOutBack",
        });
        $("folder-research").classList.add("show");
        $("nb-stats").textContent = "0 notes · 2 folders";
        toast("Folder created");
        await sleep(500);

        // New note
        caption("Add a new note to start the spatial editor");
        await clickEl($("btn-new-note"), 550);
        $("btn-new-note").classList.add("hot");

        anime.set("#dlg-note .modal", { scale: 0.9, opacity: 0 });
        $("dlg-note").classList.add("open");
        anime({
          targets: "#dlg-note .modal",
          scale: [0.9, 1],
          opacity: [0, 1],
          duration: 350,
          easing: "easeOutBack",
        });
        await sleep(300);

        await clickEl($("dlg-note-input"), 350);
        await typeInto($("dlg-note-input"), "Canvas Walkthrough", 16);
        await sleep(200);
        await clickEl($("dlg-note-ok"), 400);

        anime({
          targets: "#dlg-note .modal",
          scale: 0.9,
          opacity: 0,
          duration: 200,
          easing: "easeInQuad",
          complete: () => $("dlg-note").classList.remove("open"),
        });
        $("btn-new-note").classList.remove("hot");

        $("notes-empty").style.display = "none";
        $("note-row").style.display = "flex";
        anime({
          targets: "#note-row",
          opacity: [0, 1],
          translateY: [15, 0],
          duration: 450,
          easing: "easeOutBack",
        });
        $("note-row").classList.add("show", "hot");
        $("nb-stats").textContent = "1 note · 2 folders";
        $("folder-count").textContent = "1";
        toast("Note created");
        await sleep(500);

        caption("Launch note editor");
        await clickEl($("note-row"), 500);
        cursorShow(false);
        await sleep(250);
      }

      async function sceneCanvas() {
        setSceneLabel(3, "Canvas");
        setProgress(45);
        caption("Typewriter Markdown — write and compile interactively");
        url("samarbook.app/n/cosmic-ideas/canvas-walkthrough");

        // Place cursor in sidebar top-left area while hidden
        state.cursorX = 40;
        state.cursorY = 120;
        cursor.style.left = "40px";
        cursor.style.top = "120px";

        showScene("sc-app");
        await sleep(250);
        cursorShow(true);

        // reset canvas elements
        state.constellationActive = false;
        state.completedEdges = [];
        $("plugin-dock-buttons").style.display = "none";
        $("dlg-settings").classList.remove("open");
        $("toggle-constellation").classList.remove("active");

        // reset sidebar tree folders
        $("sb-folder-research-node").style.display = "none";
        $("sb-research-contents").style.display = "none";
        $("sb-research-chev-right").style.display = "inline-block";
        $("sb-research-chev-down").style.display = "none";
        $("sb-research-icon-closed").style.display = "inline-block";
        $("sb-research-icon-open").style.display = "none";
        $("sb-notes-contents").style.display = "block";
        $("sb-row-plugin-ideas").style.display = "flex";

        // reset cards
        const cardPositions = {
          card1: { left: "48px", top: "90px" },
          card2: { left: "290px", top: "70px" },
          card3: { left: "160px", top: "250px" },
          card4: { left: "420px", top: "230px" },
        };
        Object.keys(cardPositions).forEach((c) => {
          $(c).classList.remove("show", "active");
          $(c).style.left = cardPositions[c].left;
          $(c).style.top = cardPositions[c].top;
          $(c).style.width = "200px";
          $(c).style.minHeight = "88px";
          $(c).style.height = "auto";
          $(c).style.opacity = "0";
          $(c).style.transform = "scale(0.85) translateY(15px)";
        });

        $("ct1").innerHTML = "";
        $("ct2").innerHTML = "";
        $("ct3").innerHTML = "";
        $("ct4").innerHTML = "";

        $("edges").innerHTML = "";
        $("world").style.transform = "scale(1)";
        $("zoom-pill").textContent = "100%";
        $("zoom-pill").classList.remove("on");
        $("canvas-title").classList.add("show");
        $("canvas-date").classList.add("show");
        $("palette").classList.remove("open");

        document.querySelectorAll(".dock-btn").forEach((b) => b.classList.remove("hot"));
        $("dock-canvas").classList.add("hot");

        await sleep(300);

        // Dynamic folder reveal in sidebar explorer
        caption("Research folder synced to local repository");
        $("sb-folder-research-node").style.display = "block";
        anime({
          targets: "#sb-folder-research-node",
          opacity: [0, 1],
          translateY: [10, 0],
          duration: 400,
          easing: "easeOutQuad",
        });
        await sleep(500);

        caption("Expand folders to navigate notes");
        await clickEl($("sb-folder-research-row"), 650);

        // Toggle icons
        $("sb-research-chev-right").style.display = "none";
        $("sb-research-chev-down").style.display = "inline-block";
        $("sb-research-icon-closed").style.display = "none";
        $("sb-research-icon-open").style.display = "inline-block";
        playSound("click");

        // Expand list with Anime.js
        $("sb-research-contents").style.display = "block";
        anime({
          targets: "#sb-research-contents",
          opacity: [0, 1],
          scaleY: [0.8, 1],
          duration: 350,
          easing: "easeOutQuad",
        });
        anime({
          targets: "#sb-research-contents .sb-row",
          opacity: [0, 1],
          translateX: [-10, 0],
          delay: anime.stagger(80),
          duration: 300,
          easing: "easeOutQuad",
        });
        await sleep(1000);

        // Markdown Todo Typewriting scene
        caption("Interactive Markdown — type to compile checkboxes");
        await clickEl($("card1"), 550);
        revealCard("card1");
        $("card1").classList.add("active");

        const rawMarkdownCard1 = "### Physics Plan\n- [ ] Orbit simulation\n- [ ] Vector fields";
        const renderedHtmlCard1 = `
      <div class="md-header">Physics Plan</div>
      <div class="todo-list">
        <div class="todo-item" id="todo-orbit">
          <div class="todo-chk"><i data-lucide="check" class="size-2.5"></i></div>
          <span class="label">Orbit simulation</span>
        </div>
        <div class="todo-item" id="todo-vector">
          <div class="todo-chk"><i data-lucide="check" class="size-2.5"></i></div>
          <span class="label">Vector fields</span>
        </div>
      </div>
    `;
        await typeMarkdown($("ct1"), rawMarkdownCard1, renderedHtmlCard1, 35);
        $("card1").classList.remove("active");
        await sleep(400);

        // Ticking todo checkbox
        caption("Interact with documents in place");
        const todoChk = $("card1").querySelector("#todo-orbit");
        await clickEl(todoChk, 500);
        todoChk.classList.add("checked");
        playSound("check");
        await sleep(600);

        // Type card 2
        caption("Rich markup — headers, formulas, bold text");
        await clickEl($("card2"), 500);
        revealCard("card2");
        $("card2").classList.add("active");
        const rawMarkdownCard2 = "**Field coordinates**:\nF(x,y) = -y*i + x*j";
        const renderedHtmlCard2 = `
      <div style="font-size:11px; line-height:1.4;">
        <strong>Field coordinates</strong>:
        <div style="margin-top:4px; font-family:'JetBrains Mono', monospace; background:#f3f4f6; padding:4px 6px; border-radius:6px; border:1px solid #e5e7eb; color:var(--blue);">F(x,y) = -y*i + x*j</div>
      </div>
    `;
        await typeMarkdown($("ct2"), rawMarkdownCard2, renderedHtmlCard2, 38);
        $("card2").classList.remove("active");
        await sleep(300);

        // Type card 3
        caption("State & values — declare workspace @variables");
        await clickEl($("card3"), 500);
        revealCard("card3");
        $("card3").classList.add("active");
        const rawMarkdownCard3 = "*Variables*:\nx = @planet_mass::1.2e24";
        const renderedHtmlCard3 = `
      <div style="font-size:11px; line-height:1.4;">
        <em>Variables</em>:
        <div style="margin-top:4px; display:flex; align-items:center; gap:4px;">
          <span style="font-family:'JetBrains Mono', monospace; background:#eef2ff; color:#4f46e5; border:1.5px solid #e0e7ff; padding:2px 6px; border-radius:6px; font-size:10px; font-weight:600;">@planet_mass</span>
          <span style="color:#9ca3af;">=</span>
          <span style="font-family:'JetBrains Mono', monospace; font-weight:600; color:var(--violet);">1.2e24</span>
        </div>
      </div>
    `;
        await typeMarkdown($("ct3"), rawMarkdownCard3, renderedHtmlCard3, 38);
        $("card3").classList.remove("active");
        await sleep(300);

        // Type card 4
        caption("Wikilinks — create connected paths");
        await clickEl($("card4"), 500);
        revealCard("card4");
        $("card4").classList.add("active");
        const rawMarkdownCard4 = "[[Graph overview]] links note clusters.";
        const renderedHtmlCard4 = `
      <div style="font-size:11px; line-height:1.4;">
        <span class="wikilink" style="color:var(--violet); font-weight:600; cursor:pointer; text-decoration:underline; text-underline-offset:2px;"><i data-lucide="network" class="size-3" style="margin-right:2px; display:inline-block;"></i>Graph overview</span> links note clusters.
      </div>
    `;
        await typeMarkdown($("ct4"), rawMarkdownCard4, renderedHtmlCard4, 38);
        $("card4").classList.remove("active");
        await sleep(400);

        // Edges (interactive connection dragging)
        caption("Drag ports manually to connect related canvas cards");
        await connectCards("card1", "card2", "curve", "", "right", "left", 1200);
        await sleep(400);

        caption("Hold ALT to align routing lines in straight grid mode");
        await connectCards("card3", "card1", "straight", "", "left", "left", 1150);
        await sleep(400);

        caption("Hold CTRL to declare connection properties and labels");
        await connectCards("card2", "card4", "label", "references", "right", "left", 1150);
        await sleep(400);

        caption("Link cards together to construct structured canvas workspaces");
        state.completedEdges.push({ from: "card3", to: "card4", type: "curve" });
        drawEdges();
        await sleep(800);

        // Zoom out/in
        caption("Zoom viewport to fit layout");
        $("zoom-pill").classList.add("on");

        // Anime.js zoom out
        anime({
          targets: "#world",
          scale: 0.72,
          duration: 800,
          easing: "easeOutQuart",
          update: () => {
            const scaleVal = parseFloat(anime.get(document.getElementById("world"), "scale")) || 1;
            $("zoom-pill").textContent = `${Math.round(scaleVal * 100)}%`;
          },
        });
        await sleep(1000);

        // Anime.js zoom in
        anime({
          targets: "#world",
          scale: 1,
          duration: 600,
          easing: "easeOutBack",
          update: () => {
            const scaleVal = parseFloat(anime.get(document.getElementById("world"), "scale")) || 1;
            $("zoom-pill").textContent = `${Math.round(scaleVal * 100)}%`;
          },
        });
        await sleep(400);

        // CARD DRAGGING SEQUENCE
        caption("Arrange nodes freely — connections follow");
        await dragElTo("card3", 80, 290, 1100);
        await sleep(400);

        await dragElTo("card2", 340, 80, 900);
        await sleep(400);

        // CARD RESIZING SEQUENCE (WYSIWYG edit flow)
        caption("WYSIWYG layout — drag corners to resize cards");
        await resizeElTo("card1", 240, 115, 1200);
        await sleep(600);

        // PLUGINS PANEL SETUP
        caption("Expand features in settings panel");
        await clickEl($("dock-settings"), 600);

        anime.set("#dlg-settings .settings-modal", { scale: 0.94, opacity: 0, translateY: 15 });
        $("dlg-settings").classList.add("open");
        anime({
          targets: "#dlg-settings .settings-modal",
          scale: 1,
          opacity: 1,
          translateY: 0,
          duration: 400,
          easing: "easeOutBack",
        });
        await sleep(500);

        // Switch settings tab to plugins
        caption("Enable custom block arrangement APIs");
        await clickEl($("tab-btn-plugins"), 400);
        showSettingsTab("plugins");
        await sleep(550);

        // Toggle switch constellation layout plugin
        caption("Toggle Constellation Circle Layout ON");
        const toggleBtn = $("toggle-constellation");
        await clickEl(toggleBtn, 450);
        toggleBtn.classList.add("active");
        playSound("plugin");
        await sleep(700);

        // Close settings dialog
        caption("Plugin added to bottom dock");
        await clickEl($("btn-close-settings"), 400);

        anime({
          targets: "#dlg-settings .settings-modal",
          scale: 0.94,
          opacity: 0,
          translateY: 15,
          duration: 250,
          easing: "easeInQuad",
          complete: () => $("dlg-settings").classList.remove("open"),
        });
        await sleep(250);

        // Star button fades in
        $("plugin-dock-buttons").style.display = "inline-flex";
        anime({
          targets: "#dock-constellation",
          scale: [0.3, 1],
          opacity: [0, 1],
          duration: 500,
          easing: "easeOutBack",
        });
        safeCreateIcons();
        await sleep(400);

        // Click Constellation button in dock
        caption("Align cards in circular constellation layout");
        await clickEl($("dock-constellation"), 500);
        playSound("success");
        state.constellationActive = true;
        await animateCardsToCircle(380, 220, 150, 1200);
        await sleep(1500);

        // Command palette search
        caption("Use ⌘K Command Palette search");
        await clickEl($("dock-search"), 550);
        $("dock-search").classList.add("hot");
        $("dock-canvas").classList.remove("hot");

        anime.set("#palette", { scale: 0.93, opacity: 0 });
        $("palette").classList.add("open");
        anime({
          targets: "#palette",
          scale: 1,
          opacity: 1,
          duration: 300,
          easing: "easeOutBack",
        });

        ["pi1", "pi2", "pi3", "pi4"].forEach((id) => $(id).classList.remove("show", "sel"));
        $("palette-q").textContent = "";
        await sleep(250);

        for (const id of ["pi1", "pi2", "pi3", "pi4"]) {
          $(id).classList.add("show");
          await sleep(80);
        }

        await typeIntoTextNode($("palette-q"), "graph", 14);
        $("pi1").classList.add("sel");
        await sleep(400);

        await clickEl($("pi1"), 450);
        cursorShow(false);

        anime({
          targets: "#palette",
          scale: 0.93,
          opacity: 0,
          duration: 200,
          easing: "easeInQuad",
          complete: () => $("palette").classList.remove("open"),
        });
        $("dock-search").classList.remove("hot");
        toast("Opening Graph view");
        await sleep(350);
      }

