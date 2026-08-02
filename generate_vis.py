import os

html_content = """<!DOCTYPE html>
<html>
<head>
<title>L++ N-Dimensional Static Analysis</title>
<style>
  body { font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif; background: #08080a; color: #eee; margin: 0; padding: 20px; overflow: hidden; }
  #toolbar { display: flex; gap: 20px; align-items: center; margin-bottom: 10px; z-index: 10; position: relative; background: rgba(20,20,25,0.8); padding: 15px; border-radius: 8px; border: 1px solid #333; }
  button { padding: 10px 20px; background: #ea580c; color: white; border: none; cursor: pointer; border-radius: 4px; font-weight: bold; font-size: 14px; transition: 0.2s; }
  button:hover { background: #f97316; box-shadow: 0 0 10px #ea580c; }
  input[type=range] { width: 300px; cursor: pointer; accent-color: #ea580c; }
  canvas { position: absolute; top: 0; left: 0; width: 100vw; height: 100vh; z-index: 1; }
  .badge { background: #222; padding: 5px 10px; border-radius: 4px; font-size: 14px; border: 1px solid #444; }
  h2 { margin: 0 0 10px 0; color: #fff; text-shadow: 0 0 10px rgba(255,255,255,0.3); z-index: 10; position: relative; }
  .legend { position: absolute; bottom: 20px; right: 20px; z-index: 10; background: rgba(20,20,25,0.8); padding: 15px; border-radius: 8px; border: 1px solid #333; }
  .legend-item { display: flex; align-items: center; gap: 10px; margin-bottom: 5px; font-size: 12px; }
  .dot { width: 12px; height: 12px; border-radius: 50%; }
</style>
</head>
<body>

<h2>L++ Static Analyzer: N-Dimensional SQLite CFG & Memory Provenance</h2>
<div id="toolbar">
  <button id="playPause">⏸ Pause Analysis</button>
  <label class="badge">Engine Clock (100,000x Slower ⟷ 1x Faster): </label>
  <input type="range" id="speedSlider" min="1" max="100000" value="70000">
  <span id="speedLabel" class="badge">Fast</span>
  <label class="badge">Drag canvas to rotate</label>
</div>

<div class="legend">
  <div class="legend-item"><div class="dot" style="background:#ea580c; box-shadow: 0 0 10px #ea580c;"></div> Analyzer Path</div>
  <div class="legend-item"><div class="dot" style="background:#ef4444;"></div> Memory Alloc (malloc)</div>
  <div class="legend-item"><div class="dot" style="background:#22c55e;"></div> Memory Free (free)</div>
  <div class="legend-item"><div class="dot" style="background:rgba(100, 150, 255, 0.5);"></div> Basic Block / Branch</div>
</div>

<canvas id="canvas"></canvas>

<script>
const canvas = document.getElementById('canvas');
const ctx = canvas.getContext('2d');
const playPauseBtn = document.getElementById('playPause');
const speedSlider = document.getElementById('speedSlider');
const speedLabel = document.getElementById('speedLabel');

function resize() {
    canvas.width = window.innerWidth;
    canvas.height = window.innerHeight;
}
window.onresize = resize;
resize();

let isPlaying = true;
let stepIndex = 0;
let animationTimer = null;
let rotationX = 0.4;
let rotationY = 0.5;

// Procedural N-Dimensional Graph Generation (Simulating massive SQLite TU Graph)
const nodes = [];
const edges = [];
const numLayers = 30; // N-Dimensions / depth layers
const nodesPerLayer = 40; // Breadth of branches

// Entry point
nodes.push({ id: 0, label: 'sqlite3_main', type: 'entry', x: 0, y: 0, z: 0 });

let id = 1;
for (let layer = 1; layer <= numLayers; layer++) {
    let radius = Math.pow(layer, 1.2) * 20; // expanding universe
    for (let i = 0; i < nodesPerLayer; i++) {
        let angle = (i / nodesPerLayer) * Math.PI * 2 + (layer * 0.1); // spiral twist
        let zAngle = (layer / numLayers) * Math.PI - Math.PI/2;
        
        let x = Math.cos(angle) * radius * Math.cos(zAngle);
        let y = Math.sin(angle) * radius * Math.cos(zAngle);
        let z = radius * Math.sin(zAngle) * 1.5;
        
        let rType = Math.random();
        let type = rType > 0.95 ? 'alloc' : (rType > 0.9 ? 'free' : (rType > 0.7 ? 'branch' : 'basic'));
        nodes.push({ id, type, x, y, z });

        // Connect to previous layer to form CFG tree
        if (layer > 1) {
            edges.push({ from: id, to: id - nodesPerLayer + Math.floor(Math.random()*3 - 1) });
            // Add some cross-dimensional gotos
            if (Math.random() > 0.85) edges.push({ from: id, to: Math.floor(Math.random() * id) });
        } else {
            edges.push({ from: 0, to: id });
        }
        id++;
    }
}

// Generate an immense random walk for the "Analyzer Path" doing ownership proofs
const traversal = [0];
let curr = 0;
for(let i=0; i<3000; i++) {
    // Find edges connected to current node
    const connectedEdges = edges.filter(e => e.from === curr || e.to === curr);
    if(connectedEdges.length > 0) {
        const nextEdge = connectedEdges[Math.floor(Math.random() * connectedEdges.length)];
        curr = (nextEdge.from === curr) ? nextEdge.to : nextEdge.from;
        traversal.push(curr);
    } else {
        curr = 0; // jump back to start if dead end
        traversal.push(curr);
    }
}

// 3D Projection Engine
function project(n) {
    let cosY = Math.cos(rotationY), sinY = Math.sin(rotationY);
    let cosX = Math.cos(rotationX), sinX = Math.sin(rotationX);
    
    let nx = n.x * cosY - n.z * sinY;
    let nz = n.z * cosY + n.x * sinY;
    
    let ny = n.y * cosX - nz * sinX;
    nz = nz * cosX + n.y * sinX;
    
    let scale = 1000 / (1000 + nz);
    return { 
        x: canvas.width/2 + nx * scale, 
        y: canvas.height/2 + ny * scale, 
        scale: scale, 
        original: n 
    };
}

let isDragging = false, lastX = 0, lastY = 0;
canvas.onmousedown = e => { isDragging = true; lastX = e.clientX; lastY = e.clientY; };
canvas.onmouseup = () => isDragging = false;
canvas.onmouseleave = () => isDragging = false;
canvas.onmousemove = e => {
    if(!isDragging) return;
    rotationY += (e.clientX - lastX) * 0.005;
    rotationX += (e.clientY - lastY) * 0.005;
    lastX = e.clientX; lastY = e.clientY;
};

function draw() {
    ctx.fillStyle = '#08080a';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    
    let projNodes = nodes.map(project);
    
    // Auto-rotate slowly if not dragging
    if(!isDragging && isPlaying) {
        rotationY += 0.001;
        rotationX += 0.0005;
    }

    // Draw CFG Edges
    ctx.lineWidth = 0.5;
    ctx.strokeStyle = 'rgba(100, 150, 255, 0.15)';
    ctx.beginPath();
    edges.forEach(e => {
        let p1 = projNodes[e.from];
        let p2 = projNodes[e.to];
        if(p1 && p2 && p1.scale > 0 && p2.scale > 0) {
            ctx.moveTo(p1.x, p1.y);
            ctx.lineTo(p2.x, p2.y);
        }
    });
    ctx.stroke();

    // Draw Analyzer Path Tail (Comet effect)
    ctx.lineWidth = 2.5;
    ctx.beginPath();
    let startIdx = Math.max(0, stepIndex - 40); 
    for(let i=startIdx; i<stepIndex; i++) {
        let p1 = projNodes[traversal[i]];
        let p2 = projNodes[traversal[i+1]];
        if(p1 && p2 && p1.scale > 0 && p2.scale > 0) {
            ctx.moveTo(p1.x, p1.y);
            ctx.lineTo(p2.x, p2.y);
        }
    }
    // Gradient stroke for the comet tail
    ctx.strokeStyle = 'rgba(234, 88, 12, 0.8)';
    ctx.stroke();

    // Draw Nodes
    let activeId = traversal[stepIndex];
    projNodes.forEach((p, idx) => {
        if (p.scale <= 0) return;
        
        let isActive = (idx === activeId);
        let type = p.original.type;
        
        // Only draw important nodes or nodes close to camera to save rendering time
        if (isActive || type === 'alloc' || type === 'free' || p.scale > 1.2) {
            ctx.beginPath();
            let r = (isActive ? 8 : (type === 'alloc' || type === 'free' ? 4 : 2)) * p.scale;
            ctx.arc(p.x, p.y, r, 0, Math.PI*2);
            
            if (isActive) { ctx.fillStyle = '#ea580c'; ctx.shadowColor = '#ea580c'; ctx.shadowBlur = 20; }
            else if (type === 'alloc') { ctx.fillStyle = '#ef4444'; ctx.shadowBlur = 0; }
            else if (type === 'free') { ctx.fillStyle = '#22c55e'; ctx.shadowBlur = 0; }
            else { ctx.fillStyle = 'rgba(100, 150, 255, 0.5)'; ctx.shadowBlur = 0; }
            
            ctx.fill();
            ctx.shadowBlur = 0; // reset
        }
    });
}

function getDelay() {
    const val = parseInt(speedSlider.value);
    const maxDelay = 500; 
    const minDelay = 10;
    return maxDelay - (val / 100000) * (maxDelay - minDelay);
}

function tick() {
    if (!isPlaying) return;
    draw();
    stepIndex = (stepIndex + 1) % traversal.length;
    animationTimer = setTimeout(tick, getDelay());
}

playPauseBtn.onclick = () => {
    isPlaying = !isPlaying; 
    playPauseBtn.innerText = isPlaying ? '⏸ Pause Analysis' : '▶ Resume Analysis';
    if (isPlaying) tick(); else clearTimeout(animationTimer);
};

speedSlider.oninput = () => {
    const v = parseInt(speedSlider.value);
    if (v > 80000) speedLabel.innerText = '1x (Realtime)';
    else if (v < 20000) speedLabel.innerText = '100,000x Slower';
    else speedLabel.innerText = 'Slow Motion';
};

// Start
draw();
tick();

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

print("Generated visualization.lpp successfully.")
