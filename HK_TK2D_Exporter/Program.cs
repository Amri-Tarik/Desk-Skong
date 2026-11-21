// HK_TK2D_Exporter — dumps tk2d sprite collections + sprite animation libraries
// from a Hollow Knight install into plain JSON (no art is copied).
//
//   dotnet run -c Release -- --data "<...>/hollow_knight_Data" --list
//   dotnet run -c Release -- --data "<...>/hollow_knight_Data" --out ../data/tk2d \
//        [--collections Knight,Sprint] [--animations Knight]
//   dotnet run -c Release -- --data "<...>/hollow_knight_Data" --dump-mono HeroController
//        [--files resources.assets,level1] [--max-array 64]
//
// Collections/animations are matched by m_Name (case-insensitive). Without filters,
// every collection referenced by the exported animation libraries is exported.
using System.Text.Json;
using System.Text.Json.Serialization;
using AssetsTools.NET;
using AssetsTools.NET.Extra;
using AssetsTools.NET.Texture;

internal static class Program
{
    sealed record Options(string DataDir, string? OutDir, bool List, HashSet<string>? Collections, HashSet<string>? Animations, string[] Files, string? DumpMono, int MaxArray, string? TexturesDir, string? Find, HashSet<string>? ExportSprites, bool Hierarchy, bool Root);

    static int Main(string[] args)
    {
        Options opt;
        try { opt = ParseArgs(args); }
        catch (Exception e) { Console.Error.WriteLine(e.Message); PrintUsage(); return 2; }

        var managed = Path.Combine(opt.DataDir, "Managed");
        if (!Directory.Exists(managed)) { Console.Error.WriteLine($"Managed dir not found: {managed}"); return 4; }

        var am = new AssetsManager { MonoTempGenerator = new MonoCecilTempGenerator(managed) };
        var tpk = new[] { Path.Combine(AppContext.BaseDirectory, "classdata.tpk"), Path.Combine(Directory.GetCurrentDirectory(), "classdata.tpk") }
            .FirstOrDefault(File.Exists);
        if (tpk is null) { Console.Error.WriteLine("classdata.tpk not found"); return 3; }
        am.LoadClassPackage(tpk);

        if (opt.Find is not null)
            return Find(am, opt);
        if (opt.ExportSprites is not null)
            return opt.TexturesDir is null ? Fail("--export-sprites needs --textures <dir>") : Find(am, opt);

        var scan = new Scanner(am) { DumpClass = opt.DumpMono };
        foreach (var f in opt.Files)
        {
            var path = Path.Combine(opt.DataDir, f);
            if (!File.Exists(path)) { Console.Error.WriteLine($"skip missing {path}"); continue; }
            scan.Scan(path);
        }

        if (opt.DumpMono is not null)
            return DumpMono(am, scan, opt);

        if (opt.List)
        {
            Console.WriteLine("== tk2dSpriteCollectionData ==");
            foreach (var c in scan.Collections.OrderBy(c => c.Name))
                Console.WriteLine($"  {c.Name,-40} sprites:{c.Field["spriteDefinitions.Array"].AsArray.size,5}  file:{Path.GetFileName(c.Afi.path)}");
            Console.WriteLine("== tk2dSpriteAnimation ==");
            foreach (var a in scan.Animations.OrderBy(a => a.Name))
                Console.WriteLine($"  {a.Name,-40} clips:{a.Field["clips.Array"].AsArray.size,5}  file:{Path.GetFileName(a.Afi.path)}");
            return 0;
        }

        if (opt.OutDir is null) { PrintUsage(); return 2; }
        Directory.CreateDirectory(Path.Combine(opt.OutDir, "collections"));
        Directory.CreateDirectory(Path.Combine(opt.OutDir, "animations"));

        var json = new JsonSerializerOptions
        {
            WriteIndented = true,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
            PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        };

        // --- animations first: they tell us which collections matter
        var neededCollections = new HashSet<long>();
        var anims = scan.Animations.Where(a => opt.Animations is null || opt.Animations.Contains(a.Name)).ToList();
        foreach (var a in anims)
        {
            var lib = ExportAnimation(am, scan, a, neededCollections);
            var p = Path.Combine(opt.OutDir, "animations", Safe(a.Name) + ".json");
            File.WriteAllText(p, JsonSerializer.Serialize(lib, json));
            Console.WriteLine($"animation  {a.Name}: {lib.Clips.Count} clips -> {p}");
        }

        var cols = scan.Collections.Where(c =>
            opt.Collections is not null ? opt.Collections.Contains(c.Name) : neededCollections.Contains(c.Key)).ToList();
        foreach (var c in cols)
        {
            var col = ExportCollection(am, c);
            var p = Path.Combine(opt.OutDir, "collections", Safe(c.Name) + ".json");
            File.WriteAllText(p, JsonSerializer.Serialize(col, json));
            Console.WriteLine($"collection {c.Name}: {col.Sprites.Count} sprites, atlas {col.AtlasName} {col.AtlasWidth}x{col.AtlasHeight} -> {p}");
            if (opt.TexturesDir is not null) ExportAtlas(am, c, opt.TexturesDir);
        }
        return 0;
    }

    // ------------------------------------------------------------------ find

    /// List Sprite / Texture2D assets whose name contains `opt.Find`, across resources and
    /// every sharedassets file (or `--files` if given explicitly).
    static int Find(AssetsManager am, Options opt)
    {
        var files = opt.Files.Length == 1 && opt.Files[0] == "resources.assets"
            ? Directory.GetFiles(opt.DataDir, "sharedassets*.assets").Prepend(Path.Combine(opt.DataDir, "resources.assets"))
            : opt.Files.Select(f => Path.Combine(opt.DataDir, f));
        var needle = opt.Find ?? "";
        var wanted = opt.ExportSprites;
        foreach (var path in files)
        {
            AssetsFileInstance afi;
            try { afi = am.LoadAssetsFile(path, false); am.LoadClassDatabaseFromPackage(afi.file.Metadata.UnityVersion); }
            catch (Exception e) { Console.Error.WriteLine($"skip {Path.GetFileName(path)}: {e.Message}"); continue; }
            foreach (var (type, label) in new[] { (AssetClassID.Sprite, "Sprite"), (AssetClassID.Texture2D, "Texture2D") })
            {
                foreach (var info in afi.file.GetAssetsOfType(type))
                {
                    try
                    {
                        var bf = am.GetBaseField(afi, info);
                        var name = bf["m_Name"].AsString ?? "";
                        if (wanted is not null)
                        {
                            if (type == AssetClassID.Sprite && wanted.Remove(name)) ExportSprite(am, afi, bf, name, opt.TexturesDir!);
                            continue;
                        }
                        if (!name.Contains(needle, StringComparison.OrdinalIgnoreCase)) continue;
                        var size = type == AssetClassID.Texture2D
                            ? $"{bf["m_Width"].AsInt}x{bf["m_Height"].AsInt}"
                            : $"{bf["m_Rect"]["width"].AsFloat}x{bf["m_Rect"]["height"].AsFloat}";
                        Console.WriteLine($"{Path.GetFileName(path),-24} {label,-9} {info.PathId,20} {size,11}  {name}");
                    }
                    catch { }
                }
            }
            am.UnloadAssetsFile(afi);
            if (wanted is { Count: 0 }) break;
        }
        if (wanted is { Count: > 0 }) Console.Error.WriteLine($"not found: {string.Join(", ", wanted)}");
        return 0;
    }

    static int Fail(string msg) { Console.Error.WriteLine(msg); return 2; }

    /// For sprites packed into a SpriteAtlas: the atlas's render data (texture + textureRect)
    /// matching the sprite's `m_RenderDataKey`.
    static AssetTypeValueField? AtlasRenderData(AssetsManager am, AssetsFileInstance afi, AssetTypeValueField sprite)
    {
        var atlas = am.GetExtAsset(afi, sprite["m_SpriteAtlas"]).baseField;
        if (atlas is null) return null;
        var key = sprite["m_RenderDataKey"];
        string Key(AssetTypeValueField k) =>
            string.Join(",", k["first"].Children.Select(c => c.AsUInt.ToString())) + ":" + k["second"].AsLong;
        var want = Key(key);
        foreach (var entry in atlas["m_RenderDataMap.Array"].Children)
            if (Key(entry["first"]) == want)
                return entry["second"];
        return null;
    }

    /// Crop a Unity Sprite out of its texture and save `<name>.png` + `<name>.json` (pivot,
    /// pixels-per-unit). Game art: keep it in the git-ignored skins folder.
    static void ExportSprite(AssetsManager am, AssetsFileInstance afi, AssetTypeValueField sprite, string name, string dir)
    {
        try
        {
            var rd = sprite["m_RD"];
            if (rd["texture"]["m_PathID"].AsLong == 0)
                rd = AtlasRenderData(am, afi, sprite) ?? throw new InvalidOperationException("sprite has no texture and no atlas entry");
            var tex = am.GetExtAsset(afi, rd["texture"]);
            var tf = TextureFile.ReadTextureFile(tex.baseField);
            var raw = tf.FillPictureData(Path.GetDirectoryName(tex.file.path) ?? ".");
            var rgba = tf.DecodeTextureRaw(raw, false);
            var r = rd["textureRect"];
            int x = (int)MathF.Round(r["x"].AsFloat), y = (int)MathF.Round(r["y"].AsFloat);
            int w = (int)MathF.Round(r["width"].AsFloat), h = (int)MathF.Round(r["height"].AsFloat);
            int tw = tf.m_Width, th = tf.m_Height;
            var outPx = new byte[w * h * 4];
            // Texture rows are bottom-up; rect origin is bottom-left. Output top-down.
            for (int row = 0; row < h; row++)
                Buffer.BlockCopy(rgba, ((y + h - 1 - row) * tw + x) * 4, outPx, row * w * 4, w * 4);
            // The packer may have flipped the sprite to fit it in (settingsRaw bits 2-5).
            uint rotation = (rd["settingsRaw"].AsUInt >> 2) & 0xF;
            if (rotation > 3) throw new InvalidOperationException($"packed rotated ({rotation}), not supported");
            Unflip(outPx, w, h, flipX: rotation is 1 or 3, flipY: rotation is 2 or 3);
            ApplyMeshMask(sprite, w, h, outPx);
            Directory.CreateDirectory(dir);
            using (var fs = File.Create(Path.Combine(dir, name + ".png")))
                new StbImageWriteSharp.ImageWriter().WritePng(outPx, w, h, StbImageWriteSharp.ColorComponents.RedGreenBlueAlpha, fs);
            var pivot = sprite["m_Pivot"];
            File.WriteAllText(Path.Combine(dir, name + ".json"), JsonSerializer.Serialize(new { name, width = w, height = h, pivot = new[] { pivot["x"].AsFloat, pivot["y"].AsFloat }, pixels_per_unit = sprite["m_PixelsToUnits"].AsFloat }));
            Console.WriteLine($"sprite {name} {w}x{h} (texture {tw}x{th}) -> {dir}");
        }
        catch (Exception e) { Console.Error.WriteLine($"sprite {name}: {e.Message}"); }
    }

    /// Undo a packer flip in place (top-down RGBA, `w` x `h`).
    static void Unflip(byte[] px, int w, int h, bool flipX, bool flipY)
    {
        if (!flipX && !flipY) return;
        var src = (byte[])px.Clone();
        for (int row = 0; row < h; row++)
            for (int col = 0; col < w; col++)
            {
                int sr = flipY ? h - 1 - row : row, sc = flipX ? w - 1 - col : col;
                Buffer.BlockCopy(src, (sr * w + sc) * 4, px, (row * w + col) * 4, 4);
            }
    }

    /// Tight-packed sprites share their rectangle with neighbours in the texture: only the
    /// sprite's mesh is its own. Clear every pixel whose centre is outside all of the mesh's
    /// triangles, after growing the mesh by one pixel (8-neighbourhood) so antialiased edges
    /// stay. Same rule as `skong_extract::sprite::mesh_mask` (the two must match pixel for
    /// pixel). Sprites without a readable float position channel are left as they are.
    static void ApplyMeshMask(AssetTypeValueField sprite, int w, int h, byte[] px)
    {
        try
        {
            var rd = sprite["m_RD"];
            var vd = rd["m_VertexData"];
            int count = (int)vd["m_VertexCount"].AsUInt;
            var channels = vd["m_Channels.Array"].Children;
            if (count == 0 || channels.Count == 0) return;
            var pos = channels[0];
            if (pos["stream"].AsByte != 0 || pos["format"].AsByte != 0 || (pos["dimension"].AsByte & 0xF) < 2) return;
            int stride = 0;
            foreach (var c in channels)
            {
                if (c["stream"].AsByte != 0 || (c["dimension"].AsByte & 0xF) == 0) continue;
                int size = c["format"].AsByte switch { 0 or 10 or 11 => 4, 1 or 4 or 5 or 8 or 9 => 2, _ => 1 };
                stride = Math.Max(stride, c["offset"].AsByte + size * (c["dimension"].AsByte & 0xF));
            }
            var data = vd["m_DataSize"].AsByteArray;
            var arr = rd["m_IndexBuffer"]["Array"];
            var idx = arr.Value?.ValueType == AssetValueType.ByteArray ? arr.AsByteArray : arr.Children.Select(b => b.AsByte).ToArray();
            if (data.Length < count * stride || idx.Length < 6) return;
            double ppu = sprite["m_PixelsToUnits"].AsFloat;
            var rect = sprite["m_Rect"]; var piv = sprite["m_Pivot"]; var off = rd["textureRectOffset"];
            double bx = piv["x"].AsFloat * (double)rect["width"].AsFloat - off["x"].AsFloat;
            double by = piv["y"].AsFloat * (double)rect["height"].AsFloat - off["y"].AsFloat;
            var vx = new double[count]; var vy = new double[count];
            for (int i = 0; i < count; i++)
            {
                int o = i * stride + pos["offset"].AsByte;
                vx[i] = BitConverter.ToSingle(data, o) * ppu + bx;
                vy[i] = BitConverter.ToSingle(data, o + 4) * ppu + by;
            }
            var inside = new bool[w * h];
            for (int t = 0; t + 5 < idx.Length; t += 6)
            {
                int a = BitConverter.ToUInt16(idx, t), b = BitConverter.ToUInt16(idx, t + 2), c = BitConverter.ToUInt16(idx, t + 4);
                if (a >= count || b >= count || c >= count) return;
                for (int row = 0; row < h; row++)
                    for (int col = 0; col < w; col++)
                    {
                        if (inside[row * w + col]) continue;
                        double qx = col + 0.5, qy = h - row - 0.5;
                        double e0 = (vx[b] - vx[a]) * (qy - vy[a]) - (vy[b] - vy[a]) * (qx - vx[a]);
                        double e1 = (vx[c] - vx[b]) * (qy - vy[b]) - (vy[c] - vy[b]) * (qx - vx[b]);
                        double e2 = (vx[a] - vx[c]) * (qy - vy[c]) - (vy[a] - vy[c]) * (qx - vx[c]);
                        if ((e0 >= 0 && e1 >= 0 && e2 >= 0) || (e0 <= 0 && e1 <= 0 && e2 <= 0)) inside[row * w + col] = true;
                    }
            }
            for (int row = 0; row < h; row++)
                for (int col = 0; col < w; col++)
                {
                    bool keep = false;
                    for (int dy = -1; dy <= 1 && !keep; dy++)
                        for (int dx = -1; dx <= 1 && !keep; dx++)
                        {
                            int r2 = row + dy, c2 = col + dx;
                            keep = r2 >= 0 && r2 < h && c2 >= 0 && c2 < w && inside[r2 * w + c2];
                        }
                    if (!keep) { int o = (row * w + col) * 4; px[o] = px[o + 1] = px[o + 2] = px[o + 3] = 0; }
                }
        }
        catch (Exception e) { Console.Error.WriteLine($"  mesh mask skipped: {e.Message}"); }
    }

    // ------------------------------------------------------------------ export: atlas textures

    /// Custom-Knight skin file names, so a vanilla export is itself a valid skin folder.
    static readonly Dictionary<string, string> SkinFileNames = new(StringComparer.OrdinalIgnoreCase)
    {
        ["Knight"] = "Knight.png",
        ["Knight Dream Gate Cln"] = "Sprint.png",
        ["Knight Slug Cln"] = "Unn.png",
        ["Grimmchild Cln"] = "Grimm.png",
        ["Weaverling Cln"] = "Weaver.png",
        ["Knight Hatchling Cln"] = "Hatchling.png",
        ["Shade"] = "Shade.png",
    };

    /// Decode the collection's atlas textures and save them as PNG (top-down rows): the
    /// first as the skin file name, page k > 0 as `<name>_<k>.png` (tk2d `materialId` k).
    /// The output is game art: write it somewhere git-ignored and never redistribute it.
    static void ExportAtlas(AssetsManager am, Scanner.Entry c, string dir)
    {
        var textures = c.Field["textures.Array"];
        var main = SkinFileNames.TryGetValue(c.Name, out var n) ? n : Safe(c.Name) + ".png";
        for (int page = 0; page < textures.AsArray.size; page++)
        {
            try
            {
                var ext = am.GetExtAsset(c.Afi, textures[page]);
                var tf = TextureFile.ReadTextureFile(ext.baseField);
                var raw = tf.FillPictureData(Path.GetDirectoryName(ext.file.path) ?? ".");
                Directory.CreateDirectory(dir);
                var name = page == 0 ? main : Path.GetFileNameWithoutExtension(main) + $"_{page}.png";
                var path = Path.Combine(dir, name);
                if (raw is null || !tf.DecodeTextureImage(raw, path, ImageExportType.Png, 100))
                {
                    Console.Error.WriteLine($"  texture {c.Name} page {page}: could not decode {(TextureFormat)tf.m_TextureFormat}");
                    continue;
                }
                Console.WriteLine($"  atlas {tf.m_Width}x{tf.m_Height} {(TextureFormat)tf.m_TextureFormat} -> {path}");
            }
            catch (Exception e) { Console.Error.WriteLine($"  texture {c.Name} page {page}: {e.Message}"); }
        }
    }

    // ------------------------------------------------------------------ export: collections

    sealed class CollectionOut
    {
        public string Name { get; set; } = "";
        public string AtlasName { get; set; } = "";
        public int AtlasWidth { get; set; }
        public int AtlasHeight { get; set; }
        /// Pages 1.. of the atlas (sprites with `material_id` k use page k), if any.
        [System.Text.Json.Serialization.JsonIgnore(Condition = System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull)]
        public List<AtlasOut>? ExtraAtlases { get; set; }
        public List<SpriteOut> Sprites { get; set; } = new();
    }

    sealed class AtlasOut
    {
        public string Name { get; set; } = "";
        public int Width { get; set; }
        public int Height { get; set; }
    }

    sealed class SpriteOut
    {
        public int Id { get; set; }
        public string Name { get; set; } = "";
        /// Quad/mesh vertices in local world units (x right, y up). Origin = sprite anchor.
        public float[][] Positions { get; set; } = [];
        /// Matching atlas UVs (0..1, v up / bottom-left origin).
        public float[][] Uvs { get; set; } = [];
        public int[] Indices { get; set; } = [];
        /// tk2d FlipMode: 0 None, 1 Tk2d, 2 TPackerCW (informational; UVs already encode it).
        public int Flipped { get; set; }
        /// World units per texel.
        public float[] TexelSize { get; set; } = [];
        public float[][] Bounds { get; set; } = [];          // [center, extents]
        public float[][] UntrimmedBounds { get; set; } = []; // [center, extents]
        public int MaterialId { get; set; }
    }

    static CollectionOut ExportCollection(AssetsManager am, Scanner.Entry c)
    {
        var bf = c.Field;
        var outc = new CollectionOut { Name = c.Name };
        try
        {
            var textures = bf["textures.Array"];
            if (textures.AsArray.size > 0)
            {
                var tex = am.GetExtAsset(c.Afi, textures[0]).baseField;
                outc.AtlasName = tex["m_Name"].AsString;
                outc.AtlasWidth = tex["m_Width"].AsInt;
                outc.AtlasHeight = tex["m_Height"].AsInt;
            }
            for (int page = 1; page < textures.AsArray.size; page++)
            {
                var tex = am.GetExtAsset(c.Afi, textures[page]).baseField;
                (outc.ExtraAtlases ??= new()).Add(new AtlasOut { Name = tex["m_Name"].AsString, Width = tex["m_Width"].AsInt, Height = tex["m_Height"].AsInt });
            }
        }
        catch { /* texture lives in an unloaded file */ }

        var defs = bf["spriteDefinitions.Array"];
        for (int i = 0; i < defs.AsArray.size; i++)
        {
            var d = defs[i];
            outc.Sprites.Add(new SpriteOut
            {
                Id = i,
                Name = d["name"].AsString ?? "",
                Positions = Vec3Array(d["positions.Array"]),
                Uvs = Vec2Array(d["uvs.Array"]),
                Indices = d["indices.Array"].Children.Select(x => x.AsInt).ToArray(),
                Flipped = TryInt(d, "flipped"),
                TexelSize = Vec2(d["texelSize"]),
                Bounds = Vec3Array(d["boundsData.Array"]),
                UntrimmedBounds = Vec3Array(d["untrimmedBoundsData.Array"]),
                MaterialId = TryInt(d, "materialId"),
            });
        }
        return outc;
    }

    // ------------------------------------------------------------------ export: animations

    sealed class AnimLibOut
    {
        public string Name { get; set; } = "";
        public List<ClipOut> Clips { get; set; } = new();
    }

    sealed class ClipOut
    {
        public string Name { get; set; } = "";
        public float Fps { get; set; }
        public int LoopStart { get; set; }
        /// Loop | LoopSection | Once | PingPong | RandomFrame | RandomLoop | Single
        public string WrapMode { get; set; } = "";
        public List<FrameOut> Frames { get; set; } = new();
    }

    sealed class FrameOut
    {
        public string Collection { get; set; } = "";
        public int SpriteId { get; set; }
        public string Sprite { get; set; } = "";
        public EventOut? Event { get; set; }
    }

    sealed class EventOut
    {
        public string Info { get; set; } = "";
        public int Int { get; set; }
        public float Float { get; set; }
    }

    static readonly string[] WrapModes = ["Loop", "LoopSection", "Once", "PingPong", "RandomFrame", "RandomLoop", "Single"];

    static AnimLibOut ExportAnimation(AssetsManager am, Scanner scan, Scanner.Entry a, HashSet<long> needed)
    {
        var lib = new AnimLibOut { Name = a.Name };
        var clips = a.Field["clips.Array"];
        for (int i = 0; i < clips.AsArray.size; i++)
        {
            var c = clips[i];
            var name = c["name"].AsString ?? "";
            if (string.IsNullOrEmpty(name)) continue;
            int wrap = TryInt(c, "wrapMode");
            var clip = new ClipOut
            {
                Name = name,
                Fps = c["fps"].AsFloat,
                LoopStart = TryInt(c, "loopStart"),
                WrapMode = wrap >= 0 && wrap < WrapModes.Length ? WrapModes[wrap] : wrap.ToString(),
            };
            var frames = c["frames.Array"];
            for (int f = 0; f < frames.AsArray.size; f++)
            {
                var fr = frames[f];
                var colRef = scan.Resolve(a.Afi, fr["spriteCollection"]);
                int sid = fr["spriteId"].AsInt;
                if (colRef is not null) needed.Add(colRef.Key);
                var fo = new FrameOut
                {
                    Collection = colRef?.Name ?? "?",
                    SpriteId = sid,
                    Sprite = colRef?.SpriteName(sid) ?? "",
                };
                if (fr["triggerEvent"].AsBool)
                    fo.Event = new EventOut { Info = fr["eventInfo"].AsString ?? "", Int = fr["eventInt"].AsInt, Float = fr["eventFloat"].AsFloat };
                clip.Frames.Add(fo);
            }
            lib.Clips.Add(clip);
        }
        return lib;
    }

    // ------------------------------------------------------------------ scanning

    sealed class Scanner(AssetsManager am)
    {
        public sealed class Entry(AssetsFileInstance afi, AssetFileInfo info, AssetTypeValueField field, string name)
        {
            public AssetsFileInstance Afi { get; } = afi;
            public AssetFileInfo Info { get; } = info;
            public AssetTypeValueField Field { get; } = field;
            /// m_Name, or the owning GameObject's name (Unity 6 builds leave m_Name empty).
            public string Name { get; } = name;
            public long Key => KeyOf(Afi, Info.PathId);
            string[]? _names;
            public string SpriteName(int id)
            {
                _names ??= Field["spriteDefinitions.Array"].Children.Select(d => d["name"].AsString ?? "").ToArray();
                return id >= 0 && id < _names.Length ? _names[id] : "";
            }
        }

        public List<Entry> Collections { get; } = new();
        public List<Entry> Animations { get; } = new();
        /// MonoBehaviours of this script class are collected into <see cref="Dumped"/>.
        public string? DumpClass { get; init; }
        public List<Entry> Dumped { get; } = new();
        readonly Dictionary<long, Entry> _byKey = new();
        readonly Dictionary<string, int> _fileIds = new(StringComparer.OrdinalIgnoreCase);

        static long KeyOf(AssetsFileInstance afi, long pathId) => ((long)afi.path.GetHashCode() << 32) ^ pathId;

        public void Scan(string path)
        {
            var afi = am.LoadAssetsFile(path, true);
            am.LoadClassDatabaseFromPackage(afi.file.Metadata.UnityVersion);
            Console.Error.WriteLine($"scanning {Path.GetFileName(path)} (Unity {afi.file.Metadata.UnityVersion})");
            // `--dump-mono` also accepts a built-in Unity class (e.g. Physics2DSettings).
            if (DumpClass is not null && Enum.TryParse<AssetClassID>(DumpClass, true, out var builtin) && builtin != AssetClassID.MonoBehaviour)
            {
                foreach (var info in afi.file.GetAssetsOfType(builtin))
                {
                    try { Dumped.Add(new Entry(afi, info, am.GetBaseField(afi, info), builtin.ToString())); } catch { }
                }
                return;
            }
            foreach (var info in afi.file.GetAssetsOfType(AssetClassID.MonoBehaviour))
            {
                var cls = ClassName(afi, info);
                bool dump = DumpClass is not null && string.Equals(cls, DumpClass, StringComparison.OrdinalIgnoreCase);
                if (!dump && cls is not ("tk2dSpriteCollectionData" or "tk2dSpriteAnimation")) continue;
                AssetTypeValueField bf;
                try { bf = am.GetBaseField(afi, info); } catch { continue; }
                if (dump) { Dumped.Add(new Entry(afi, info, bf, NameOf(afi, bf))); continue; }
                var e = new Entry(afi, info, bf, NameOf(afi, bf));
                (cls == "tk2dSpriteAnimation" ? Animations : Collections).Add(e);
                _byKey[e.Key] = e;
            }
        }

        public Entry? Resolve(AssetsFileInstance from, AssetTypeValueField pptr)
        {
            int fileId = pptr["m_FileID"].AsInt;
            long pathId = pptr["m_PathID"].AsLong;
            if (pathId == 0) return null;
            AssetsFileInstance target = from;
            if (fileId != 0)
            {
                target = from.GetDependency(am, fileId - 1);
                if (target is null) return null;
            }
            if (_byKey.TryGetValue(KeyOf(target, pathId), out var hit)) return hit;
            try
            {
                var ext = am.GetExtAsset(from, pptr);
                if (ext.baseField is null) return null;
                var e = new Entry(ext.file, ext.info, ext.baseField, NameOf(ext.file, ext.baseField));
                _byKey[e.Key] = e;
                Collections.Add(e);
                return e;
            }
            catch { return null; }
        }

        string NameOf(AssetsFileInstance afi, AssetTypeValueField bf)
        {
            var n = bf["m_Name"].AsString;
            if (!string.IsNullOrEmpty(n)) return n;
            try
            {
                var go = am.GetExtAsset(afi, bf["m_GameObject"]).baseField;
                n = go?["m_Name"].AsString;
            }
            catch { }
            if (!string.IsNullOrEmpty(n)) return n;
            // last resort: collections carry spriteCollectionName
            try { n = bf["spriteCollectionName"].AsString; } catch { }
            return string.IsNullOrEmpty(n) ? $"unnamed_{bf["m_GameObject"]["m_PathID"].AsLong}" : n;
        }

        public string? ClassName(AssetsFileInstance afi, AssetFileInfo info)
        {
            try
            {
                var bf = am.GetBaseField(afi, info, AssetReadFlags.SkipMonoBehaviourFields);
                var script = am.GetExtAsset(afi, bf["m_Script"]).baseField;
                return script["m_ClassName"].AsString;
            }
            catch { return null; }
        }
    }

    // ------------------------------------------------------------------ dump: MonoBehaviour fields

    /// Prints every MonoBehaviour of class <c>--dump-mono</c> as JSON: its serialized fields
    /// plus the built-in components on the same GameObject (BoxCollider2D, Rigidbody2D, ...).
    /// Only serialized data is read; no game code is decompiled.
    static AssetExternal? TransformOf(AssetsManager am, AssetExternal go)
    {
        foreach (var c in go.baseField["m_Component.Array"].Children)
        {
            var ext = am.GetExtAsset(go.file, c["component"]);
            if (ext.info is not null && (AssetClassID)ext.info.TypeId is AssetClassID.Transform or AssetClassID.RectTransform) return ext;
        }
        return null;
    }

    /// One line per GameObject: name, active, local position/scale, components (with tk2d
    /// collection / animation library + default clip), then children indented.
    static void PrintTree(AssetsManager am, Scanner scan, AssetsFileInstance file, AssetTypeValueField transform, int depth)
    {
        var go = am.GetExtAsset(file, transform["m_GameObject"]);
        var p = transform["m_LocalPosition"]; var s = transform["m_LocalScale"];
        var parts = new List<string>();
        foreach (var c in go.baseField["m_Component.Array"].Children)
        {
            try
            {
                var ext = am.GetExtAsset(go.file, c["component"]);
                if (ext.info is null) continue;
                var type = (AssetClassID)ext.info.TypeId;
                if (type is AssetClassID.Transform or AssetClassID.RectTransform) continue;
                if (type == AssetClassID.SpriteRenderer)
                {
                    var sprite = "";
                    try { var sp = am.GetExtAsset(ext.file, am.GetBaseField(ext.file, ext.info)["m_Sprite"]).baseField; if (sp is not null) sprite = $" sprite={sp["m_Name"].AsString}"; } catch { }
                    parts.Add(type + sprite);
                    continue;
                }
                if (type != AssetClassID.MonoBehaviour) { parts.Add(type.ToString()); continue; }
                var cls = scan.ClassName(ext.file, ext.info) ?? "?";
                var detail = "";
                if (cls is "tk2dSprite" or "tk2dSpriteAnimator" or "tk2dSlicedSprite" or "tk2dTiledSprite")
                {
                    var bf = am.GetBaseField(ext.file, ext.info);
                    try { var col = am.GetExtAsset(ext.file, bf["collection"]).baseField; if (col is not null) detail += $" col={NameOrGo(am, ext.file, col)} sprite={bf["_spriteId"].AsInt}"; } catch { }
                    try
                    {
                        var lib = am.GetExtAsset(ext.file, bf["library"]);
                        if (lib.baseField is not null)
                        {
                            int clip = bf["defaultClipId"].AsInt;
                            var clips = lib.baseField["clips.Array"];
                            var clipName = clip >= 0 && clip < clips.AsArray.size ? clips[clip]["name"].AsString : "?";
                            detail += $" lib={NameOrGo(am, lib.file, lib.baseField)} clip=\"{clipName}\" autoplay={bf["playAutomatically"].AsBool}";
                        }
                    }
                    catch { }
                }
                parts.Add(cls + detail);
            }
            catch { }
        }
        var active = go.baseField["m_IsActive"].AsBool ? "" : " (inactive)";
        var r = transform["m_LocalRotation"];
        var rot = RotationText(r["x"].AsFloat, r["y"].AsFloat, r["z"].AsFloat, r["w"].AsFloat);
        Console.WriteLine($"{new string(' ', depth * 2)}{go.baseField["m_Name"].AsString}{active} pos=({p["x"].AsFloat:0.###},{p["y"].AsFloat:0.###},{p["z"].AsFloat:0.###}) scale=({s["x"].AsFloat:0.###},{s["y"].AsFloat:0.###}){rot} [{string.Join("; ", parts)}]");
        foreach (var child in transform["m_Children.Array"].Children)
        {
            var ct = am.GetExtAsset(file, child);
            if (ct.baseField is not null) PrintTree(am, scan, ct.file, ct.baseField, depth + 1);
        }
    }

    /// " rot=(x,y,z)" Euler degrees (Unity ZXY order) of a local rotation, or "" for identity.
    static string RotationText(float x, float y, float z, float w)
    {
        if (Math.Abs(x) < 1e-5 && Math.Abs(y) < 1e-5 && Math.Abs(z) < 1e-5) return "";
        double D(double r) => r * 180.0 / Math.PI;
        double sx = Math.Clamp(2.0 * (w * x - y * z), -1.0, 1.0);
        double ex = Math.Asin(sx);
        double ey = Math.Atan2(2.0 * (w * y + x * z), 1.0 - 2.0 * (x * x + y * y));
        double ez = Math.Atan2(2.0 * (w * z + x * y), 1.0 - 2.0 * (x * x + z * z));
        return $" rot=({D(ex):0.#},{D(ey):0.#},{D(ez):0.#})";
    }

    static string NameOrGo(AssetsManager am, AssetsFileInstance file, AssetTypeValueField mb)
    {
        var n = mb["m_Name"].AsString;
        if (!string.IsNullOrEmpty(n)) return n;
        try { return am.GetExtAsset(file, mb["m_GameObject"]).baseField["m_Name"].AsString; } catch { return "?"; }
    }

    static int DumpMono(AssetsManager am, Scanner scan, Options opt)
    {
        if (scan.Dumped.Count == 0)
        {
            Console.Error.WriteLine($"no MonoBehaviour of class {opt.DumpMono} in {string.Join(",", opt.Files)}");
            return 5;
        }
        if (opt.Hierarchy)
        {
            var printedRoots = new HashSet<(string, long)>();
            foreach (var e in scan.Dumped)
            {
                var go = am.GetExtAsset(e.Afi, e.Field["m_GameObject"]);
                var tr = TransformOf(am, go);
                if (tr is null) continue;
                var top = tr.Value;
                // --root: print from the top of the object's tree (its prefab or scene root), once.
                while (opt.Root)
                {
                    var father = am.GetExtAsset(top.file, top.baseField["m_Father"]);
                    if (father.baseField is null) break;
                    top = father;
                }
                if (opt.Root && !printedRoots.Add((top.file.path, top.info.PathId))) continue;
                PrintTree(am, scan, top.file, top.baseField, 0);
            }
            return 0;
        }
        var result = new List<object?>();
        foreach (var e in scan.Dumped)
        {
            var obj = new Dictionary<string, object?>
            {
                ["file"] = Path.GetFileName(e.Afi.path),
                ["path_id"] = e.Info.PathId,
                ["game_object"] = e.Name,
                ["fields"] = ToJson(e.Field, opt.MaxArray),
            };
            var comps = new List<object?>();
            try
            {
                if (e.Field["m_GameObject"].IsDummy) { result.Add(obj); continue; }
                var go = am.GetExtAsset(e.Afi, e.Field["m_GameObject"]);
                foreach (var c in go.baseField["m_Component.Array"].Children)
                {
                    var ext = am.GetExtAsset(go.file, c["component"]);
                    if (ext.info is null || ext.baseField is null) continue;
                    var type = (AssetClassID)ext.info.TypeId;
                    if (type == AssetClassID.MonoBehaviour)
                        comps.Add(new Dictionary<string, object?> { ["type"] = "MonoBehaviour", ["class"] = scan.ClassName(ext.file, ext.info) });
                    else
                        comps.Add(new Dictionary<string, object?> { ["type"] = type.ToString(), ["fields"] = ToJson(ext.baseField, opt.MaxArray) });
                }
            }
            catch (Exception ex) { comps.Add(new Dictionary<string, object?> { ["error"] = ex.Message }); }
            obj["components"] = comps;
            result.Add(obj);
        }
        Console.WriteLine(JsonSerializer.Serialize(result, new JsonSerializerOptions { WriteIndented = true }));
        return 0;
    }

    /// Generic serialized-field tree -> JSON-friendly objects. Arrays longer than
    /// <paramref name="maxArray"/> are summarised by their length.
    static object? ToJson(AssetTypeValueField f, int maxArray)
    {
        var v = f.Value;
        if (v is not null)
        {
            switch (v.ValueType)
            {
                case AssetValueType.Bool: return v.AsBool;
                case AssetValueType.Int8: case AssetValueType.Int16: case AssetValueType.Int32: return v.AsInt;
                case AssetValueType.UInt8: case AssetValueType.UInt16: case AssetValueType.UInt32: return v.AsUInt;
                case AssetValueType.Int64: return v.AsLong;
                case AssetValueType.UInt64: return v.AsULong;
                case AssetValueType.Float: return float.IsFinite(v.AsFloat) ? v.AsFloat : v.AsFloat.ToString();
                case AssetValueType.Double: return double.IsFinite(v.AsDouble) ? v.AsDouble : v.AsDouble.ToString();
                case AssetValueType.String: return v.AsString;
                case AssetValueType.ByteArray: return new Dictionary<string, object?> { ["bytes"] = v.AsByteArray?.Length ?? 0 };
                case AssetValueType.Array:
                    if (f.Children.Count > maxArray) return new Dictionary<string, object?> { ["array_len"] = f.Children.Count };
                    return f.Children.Select(c => ToJson(c, maxArray)).ToList();
            }
        }
        if (f.Children is null || f.Children.Count == 0) return null;
        // Collapse Unity's `field.Array` wrapper into the field itself.
        if (f.Children.Count == 1 && f.Children[0].FieldName == "Array") return ToJson(f.Children[0], maxArray);
        var d = new Dictionary<string, object?>();
        foreach (var c in f.Children) d[c.FieldName] = ToJson(c, maxArray);
        return d;
    }

    // ------------------------------------------------------------------ helpers

    static float[] Vec2(AssetTypeValueField f) => [f["x"].AsFloat, f["y"].AsFloat];
    static float[][] Vec2Array(AssetTypeValueField arr) => arr.Children.Select(Vec2).ToArray();
    static float[][] Vec3Array(AssetTypeValueField arr) => arr.Children.Select(v => new[] { v["x"].AsFloat, v["y"].AsFloat, v["z"].AsFloat }).ToArray();

    static int TryInt(AssetTypeValueField f, string name)
    {
        try { var c = f[name]; return c.IsDummy ? 0 : c.AsInt; } catch { return 0; }
    }

    static string Safe(string s) => string.Concat(s.Select(ch => Path.GetInvalidFileNameChars().Contains(ch) || ch == ' ' ? '_' : ch));

    static Options ParseArgs(string[] args)
    {
        string? data = null, outDir = null; bool list = false;
        HashSet<string>? cols = null, anims = null;
        string[] files = ["resources.assets"];
        string? dumpMono = null; int maxArray = 64; string? textures = null; string? find = null; HashSet<string>? exportSprites = null; bool hierarchy = false; bool root = false;
        for (int i = 0; i < args.Length; i++)
        {
            string Next() => i + 1 < args.Length ? args[++i] : throw new ArgumentException($"missing value for {args[i]}");
            HashSet<string> Set(string v) => new(v.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries), StringComparer.OrdinalIgnoreCase);
            switch (args[i])
            {
                case "--data": data = Next(); break;
                case "--out": outDir = Next(); break;
                case "--list": list = true; break;
                case "--collections": cols = Set(Next()); break;
                case "--animations": anims = Set(Next()); break;
                case "--dump-mono": dumpMono = Next(); break;
                case "--max-array": maxArray = int.Parse(Next()); break;
                case "--textures": textures = Next(); break;
                case "--find": find = Next(); break;
                case "--export-sprites": exportSprites = Set(Next()); break;
                case "--hierarchy": hierarchy = true; break;
                case "--root": hierarchy = true; root = true; break;
                case "--files": files = Next().Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries); break;
                default: throw new ArgumentException($"unknown arg {args[i]}");
            }
        }
        if (data is null) throw new ArgumentException("--data is required");
        return new Options(data, outDir, list, cols, anims, files, dumpMono, maxArray, textures, find, exportSprites, hierarchy, root);
    }

    static void PrintUsage() => Console.Error.WriteLine(
        "usage: HK_TK2D_Exporter --data <hollow_knight_Data> (--list | --out <dir> | --dump-mono <Class>) [--collections A,B] [--animations A,B] [--files resources.assets,level1] [--max-array N] [--textures <dir>] [--find <name-substring>] [--export-sprites a,b --textures <dir>] [--dump-mono <Class> --hierarchy]");
}
