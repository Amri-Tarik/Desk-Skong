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

internal static class Program
{
    sealed record Options(string DataDir, string? OutDir, bool List, HashSet<string>? Collections, HashSet<string>? Animations, string[] Files, string? DumpMono, int MaxArray);

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
        }
        return 0;
    }

    // ------------------------------------------------------------------ export: collections

    sealed class CollectionOut
    {
        public string Name { get; set; } = "";
        public string AtlasName { get; set; } = "";
        public int AtlasWidth { get; set; }
        public int AtlasHeight { get; set; }
        public List<SpriteOut> Sprites { get; set; } = new();
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
    static int DumpMono(AssetsManager am, Scanner scan, Options opt)
    {
        if (scan.Dumped.Count == 0)
        {
            Console.Error.WriteLine($"no MonoBehaviour of class {opt.DumpMono} in {string.Join(",", opt.Files)}");
            return 5;
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
        string? dumpMono = null; int maxArray = 64;
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
                case "--files": files = Next().Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries); break;
                default: throw new ArgumentException($"unknown arg {args[i]}");
            }
        }
        if (data is null) throw new ArgumentException("--data is required");
        return new Options(data, outDir, list, cols, anims, files, dumpMono, maxArray);
    }

    static void PrintUsage() => Console.Error.WriteLine(
        "usage: HK_TK2D_Exporter --data <hollow_knight_Data> (--list | --out <dir> | --dump-mono <Class>) [--collections A,B] [--animations A,B] [--files resources.assets,level1] [--max-array N]");
}
