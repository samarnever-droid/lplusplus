import os

html_content = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>L++ N-Dimensional Static Analysis</title>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;800&display=swap" rel="stylesheet">
<style>
  body { 
    margin: 0; 
    overflow: hidden; 
    background-color: #030305; 
    background-image: radial-gradient(circle at 50% 50%, #0a0a12 0%, #000000 100%);
    font-family: 'Inter', sans-serif; 
    color: #e2e8f0; 
  }
  canvas { 
    position: absolute; 
    top: 0; left: 0; 
    z-index: 1; 
  }
  .glass-panel {
    position: absolute;
    z-index: 10;
    background: rgba(15, 15, 20, 0.65);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 12px;
    padding: 20px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  #ui-container {
    top: 20px;
    left: 20px;
    width: 340px;
    display: flex;
    flex-direction: column;
    gap: 15px;
  }
  .legend {
    bottom: 20px;
    right: 20px;
  }
  h2 { margin: 0 0 5px 0; font-size: 18px; font-weight: 800; background: linear-gradient(90deg, #fff, #94a3b8); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }
  p.subtitle { margin: 0 0 15px 0; font-size: 12px; color: #94a3b8; line-height: 1.4; }
  
  .control-group { display: flex; flex-direction: column; gap: 8px; }
  .control-row { display: flex; justify-content: space-between; align-items: center; font-size: 13px; font-weight: 600; }
  
  button {
    padding: 12px 20px;
    background: linear-gradient(135deg, #f97316, #ea580c);
    color: white;
    border: none;
    cursor: pointer;
    border-radius: 8px;
    font-weight: 600;
    font-size: 14px;
    transition: all 0.2s ease;
    box-shadow: 0 4px 15px rgba(234, 88, 12, 0.3);
  }
  button:hover { background: linear-gradient(135deg, #fb923c, #f97316); box-shadow: 0 6px 20px rgba(234, 88, 12, 0.4); transform: translateY(-1px); }
  button:active { transform: translateY(1px); }
  
  input[type=range] {
    -webkit-appearance: none;
    width: 100%;
    background: transparent;
  }
  input[type=range]::-webkit-slider-thumb {
    -webkit-appearance: none;
    height: 16px; width: 16px;
    border-radius: 50%;
    background: #f97316;
    cursor: pointer;
    margin-top: -6px;
    box-shadow: 0 0 10px rgba(249, 115, 22, 0.5);
  }
  input[type=range]::-webkit-slider-runnable-track {
    width: 100%; height: 4px;
    cursor: pointer;
    background: rgba(255, 255, 255, 0.1);
    border-radius: 2px;
  }
  
  .legend-item { display: flex; align-items: center; gap: 10px; margin-bottom: 8px; font-size: 12px; font-weight: 600; color: #cbd5e1; }
  .legend-item:last-child { margin-bottom: 0; }
  .dot { width: 10px; height: 10px; border-radius: 50%; }
</style>
</head>
<body>

<div id="ui-container" class="glass-panel">
  <h2>L++ Static Analyzer</h2>
  <p class="subtitle">N-Dimensional SQLite CFG & Memory Provenance. Drag canvas to rotate 3D view.</p>
  
  <button id="playPause">⏸ Pause Analysis</button>
  
  <div class="control-group" style="margin-top: 10px;">
    <div class="control-row">
      <span>Engine Clock Speed</span>
      <span id="speedLabel" style="color: #f97316;">Fast</span>
    </div>
    <input type="range" id="speedSlider" min="1" max="100" value="80">
  </div>
</div>

<div class="legend glass-panel">
  <div class="legend-item"><div class="dot" style="background:#f97316; box-shadow: 0 0 8px #f97316;"></div> Analyzer Path</div>
  <div class="legend-item"><div class="dot" style="background:#ef4444;"></div> Memory Alloc (malloc)</div>
  <div class="legend-item"><div class="dot" style="background:#22c55e;"></div> Memory Free (free)</div>
  <div class="legend-item"><div class="dot" style="background:rgba(96, 165, 250, 0.6);"></div> Basic Block / Branch</div>
</div>

<canvas id="canvas"></canvas>

<script>
const canvas = document.getElementById('canvas');
const ctx = canvas.getContext('2d', { alpha: false }); // Optimize by disabling alpha on context
const playPauseBtn = document.getElementById('playPause');
const speedSlider = document.getElementById('speedSlider');
const speedLabel = document.getElementById('speedLabel');

let width, height, cx, cy;
function resize() {
    width = canvas.width = window.innerWidth;
    height = canvas.height = window.innerHeight;
    cx = width / 2;
    cy = height / 2;
}
window.addEventListener('resize', resize);
resize();

// UI State
let isPlaying = true;
let stepIndex = 0;
let rotationX = 0.4;
let rotationY = 0.5;
let autoRotateY = 0.002;
let autoRotateX = 0.0005;

// Data Structure
const nodes = [];
const edges = [];
const numNodes = 1500; // Large but performant
const branches = 5;

// Build a beautiful 3D fractal tree to represent the CFG realistically
function buildGraph() {
    // Root
    nodes.push({ id: 0, x: 0, y: -400, z: 0, type: 'entry' });
    
    let currentLevel = [0];
    let idCounter = 1;
    let radius = 60;
    let yLevel = -300;
    
    while (idCounter < numNodes) {
        let nextLevel = [];
        let r = radius;
        
        for (let parentId of currentLevel) {
            let numChildren = Math.floor(Math.random() * branches) + 1;
            for (let i = 0; i < numChildren && idCounter < numNodes; i++) {
                let angle = Math.random() * Math.PI * 2;
                let tilt = (Math.random() - 0.5) * 0.5;
                
                let nx = nodes[parentId].x + Math.cos(angle) * r;
                let ny = yLevel + (Math.random() * 50 - 25);
                let nz = nodes[parentId].z + Math.sin(angle) * r;
                
                // Determine node type
                let rand = Math.random();
                let type = 'basic';
                if (rand > 0.96) type = 'alloc';
                else if (rand > 0.92) type = 'free';
                else if (numChildren > 1 && i === 0) type = 'branch';

                nodes.push({ id: idCounter, x: nx, y: ny, z: nz, type: type });
                edges.push({ from: parentId, to: idCounter });
                
                // Random cross-links for loops (goto/continue)
                if (Math.random() > 0.95 && idCounter > 10) {
                    edges.push({ from: idCounter, to: Math.floor(Math.random() * idCounter) });
                }
                
                nextLevel.push(idCounter);
                idCounter++;
            }
        }
        currentLevel = nextLevel;
        yLevel += 70;
        radius *= 1.1; // expand outward
        if(currentLevel.length === 0) break; // safeguard
    }
}
buildGraph();

// Pre-calculate a long random walk (Analyzer Traversal)
const traversal = [0];
let curr = 0;
for(let i = 0; i < 5000; i++) {
    // find outbound edges
    let outs = edges.filter(e => e.from === curr);
    if (outs.length === 0) {
        curr = 0; // jump to root if dead end
    } else {
        curr = outs[Math.floor(Math.random() * outs.length)].to;
    }
    traversal.push(curr);
}

// Projection cache to avoid re-allocating objects in the render loop
const projX = new Float32Array(numNodes);
const projY = new Float32Array(numNodes);
const projScale = new Float32Array(numNodes);

function project() {
    let cosY = Math.cos(rotationY), sinY = Math.sin(rotationY);
    let cosX = Math.cos(rotationX), sinX = Math.sin(rotationX);
    
    for(let i=0; i<numNodes; i++) {
        let n = nodes[i];
        let nx = n.x * cosY - n.z * sinY;
        let nz = n.z * cosY + n.x * sinY;
        
        let ny = n.y * cosX - nz * sinX;
        nz = nz * cosX + n.y * sinX;
        
        let scale = 1200 / (1200 + nz);
        projX[i] = cx + nx * scale;
        projY[i] = cy + ny * scale;
        projScale[i] = scale;
    }
}

// Input Handling
let isDragging = false, lastX = 0, lastY = 0;
canvas.addEventListener('mousedown', e => { isDragging = true; lastX = e.clientX; lastY = e.clientY; });
window.addEventListener('mouseup', () => isDragging = false);
window.addEventListener('mousemove', e => {
    if(!isDragging) return;
    rotationY += (e.clientX - lastX) * 0.005;
    rotationX += (e.clientY - lastY) * 0.005;
    lastX = e.clientX; lastY = e.clientY;
});

// Highly optimized rendering loop
function render() {
    // Update rotation
    if (!isDragging && isPlaying) {
        rotationY += autoRotateY;
        rotationX += autoRotateX;
    }
    
    project();

    // Clear with solid color (alpha: false context is faster)
    ctx.fillStyle = '#050508';
    ctx.fillRect(0, 0, width, height);

    // BATCH 1: Draw all base edges efficiently
    ctx.beginPath();
    ctx.strokeStyle = 'rgba(71, 85, 105, 0.15)'; // Slate 600 transparent
    ctx.lineWidth = 0.8;
    for(let i=0; i<edges.length; i++) {
        let f = edges[i].from;
        let t = edges[i].to;
        if (projScale[f] > 0 && projScale[t] > 0) {
            ctx.moveTo(projX[f], projY[f]);
            ctx.lineTo(projX[t], projY[t]);
        }
    }
    ctx.stroke();

    // BATCH 2: Draw the Analyzer's Comet Tail
    // Use screen compositing for a bright glowing effect without expensive shadowBlur
    ctx.globalCompositeOperation = 'screen';
    
    let startIdx = Math.max(0, stepIndex - 40);
    for(let i = startIdx; i < stepIndex; i++) {
        let f = traversal[i];
        let t = traversal[i+1];
        if (projScale[f] > 0 && projScale[t] > 0) {
            ctx.beginPath();
            ctx.moveTo(projX[f], projY[f]);
            ctx.lineTo(projX[t], projY[t]);
            
            // Tapered and fading tail
            let progress = (i - startIdx) / 40; 
            ctx.lineWidth = 3 * progress * ((projScale[f] + projScale[t])/2);
            ctx.strokeStyle = `rgba(249, 115, 22, ${progress})`; // Orange gradient
            ctx.stroke();
        }
    }
    ctx.globalCompositeOperation = 'source-over'; // Reset

    // BATCH 3: Draw Nodes grouped by color to minimize state changes
    const drawNodesByType = (typeFilter, color, sizeMultiplier) => {
        ctx.fillStyle = color;
        ctx.beginPath();
        for(let i=0; i<numNodes; i++) {
            if (projScale[i] <= 0 || nodes[i].type !== typeFilter) continue;
            // Cull small distant nodes
            if (projScale[i] < 0.3 && typeFilter === 'basic') continue; 
            
            let r = Math.max(0.5, sizeMultiplier * projScale[i]);
            ctx.moveTo(projX[i] + r, projY[i]);
            ctx.arc(projX[i], projY[i], r, 0, 6.28318);
        }
        ctx.fill();
    };

    drawNodesByType('basic', 'rgba(96, 165, 250, 0.4)', 2);
    drawNodesByType('branch', 'rgba(96, 165, 250, 0.8)', 2.5);
    drawNodesByType('alloc', '#ef4444', 3.5);
    drawNodesByType('free', '#22c55e', 3.5);

    // Draw active head
    let headId = traversal[stepIndex];
    if (projScale[headId] > 0) {
        ctx.beginPath();
        ctx.fillStyle = '#fff';
        let hr = 5 * projScale[headId];
        ctx.arc(projX[headId], projY[headId], hr, 0, 6.283);
        ctx.fill();
        
        ctx.beginPath();
        ctx.strokeStyle = 'rgba(249, 115, 22, 0.8)';
        ctx.lineWidth = 2 * projScale[headId];
        ctx.arc(projX[headId], projY[headId], hr + 4*projScale[headId], 0, 6.283);
        ctx.stroke();
    }

    requestAnimationFrame(render);
}

// Logic Tick Loop (Decoupled from render loop to ensure smooth 60FPS UI)
function logicTick() {
    if (isPlaying) {
        stepIndex = (stepIndex + 1) % traversal.length;
    }
    
    // Dynamic delay based on slider (1 = slow, 100 = fast)
    let speedVal = parseInt(speedSlider.value);
    let delay = speedVal === 100 ? 5 : (100 - speedVal) * 5; 
    
    setTimeout(logicTick, delay);
}

// UI Listeners
playPauseBtn.onclick = () => {
    isPlaying = !isPlaying; 
    playPauseBtn.innerText = isPlaying ? '⏸ Pause Analysis' : '▶ Resume Analysis';
};

speedSlider.oninput = () => {
    const v = parseInt(speedSlider.value);
    if (v > 80) speedLabel.innerText = 'Fast (Realtime)';
    else if (v < 20) speedLabel.innerText = '100,000x Slower';
    else speedLabel.innerText = 'Slow Motion';
};

// Boot
render();
logicTick();

</script>
</body>
</html>
"""

escaped_html = html_content.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n')

lpp_code = f'''# packages/lpp-analyzer/src/visualization.lpp
# Generates an interactive N-Dimensional visualization of the analyzer's Ownership and CFG graphs.

def generate_html_visualization(output_path: Str) -> Int:
    mut html := "{escaped_html}"
    return write_file(output_path, html)
'''

with open('packages/lpp-analyzer/src/visualization.lpp', 'w') as f:
    f.write(lpp_code)

print("Generated lag-free visualization.lpp successfully.")
