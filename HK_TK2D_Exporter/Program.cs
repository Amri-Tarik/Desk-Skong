using System.Text.Json;
using System.Text.Json.Serialization;
using AssetsTools.NET;
using AssetsTools.NET.Extra;

public sealed class ExportRoot
{
    public int AtlasWidth { get; set; }
    public int AtlasHeight { get; set; }
    public List<ExportSprite> Sprites { get; set; } = new();
}

public sealed class ExportSprite
{
    public string Name { get; set; } = "";
    public PixelRect PixelRect { get; set; } = new();    // atlas pixels (bottom-left origin)
    public float[] Pivot { get; set; } = new float[2];   // bounds.center in world units
    public string Flip { get; set; } = "None";           // None | Tk2d | TPackerCW
    public Size2D SourceSize { get; set; } = new();      // original (untrimmed) size in pixels
    public Offset2D SpriteSourceOffset { get; set; } = new(); // bottom-left offset of trimmed rect within untrimmed
}

public sealed class PixelRect { public int X, Y, W, H; }
public sealed class Size2D { public int W, H; }
public sealed class Offset2D { public int X, Y; }
internal class Program
{
    private static string? TryGetMonoBehaviourClassName(AssetsManager am, AssetsFileInstance afi, AssetFileInfo mbInfo)
    {
        try
        {
            var baseField = am.GetBaseField(afi, mbInfo);
            var scriptExt = am.GetExtAsset(afi, baseField["m_Script"]);
            var scriptBF = scriptExt.baseField;
            var nspace = scriptBF["m_Namespace"].AsString;
            var cname = scriptBF["m_ClassName"].AsString;
            return string.IsNullOrEmpty(nspace) ? cname : $"{nspace}.{cname}";
        }
        catch { return null; }
    }

    static int Main()
    {
        // --- Hardcoded paths (relative to project root) ---
        // When running from bin/Debug|Release/netX.Y, go up 3 levels to project root.
        var exeBase = AppContext.BaseDirectory;
        var projRoot = Path.GetFullPath(Path.Combine(exeBase, "..", "..", ".."));

        // Your layout under HK_TK2D_Exporter/
        var assetsPath = Path.Combine(projRoot, "assets", "resources.assets");
        var managedDir = Path.Combine(projRoot, "Managed");
        var outJsonPath = Path.Combine(projRoot, "Knight_tk2d.json");

        if (!File.Exists(assetsPath))
        {
            Console.Error.WriteLine($"assets not found: {assetsPath}");
            return 3;
        }
        if (!Directory.Exists(managedDir))
        {
            Console.Error.WriteLine($"Managed dir not found: {managedDir}");
            return 4;
        }

        var am = new AssetsManager();

        am.MonoTempGenerator = new MonoCecilTempGenerator(managedDir);

        // Optional: load classdata.tpk if you have it next to the project or exe (improves non-Mono types like Texture2D)
        var tpkCandidates = new[] {
            Path.Combine(projRoot, "classdata.tpk"),
            Path.Combine(exeBase, "classdata.tpk")
        };
        foreach (var tpk in tpkCandidates)
        {
            if (File.Exists(tpk))
            {
                am.LoadClassPackage(tpk);
                break;
            }
        }

        var afi = am.LoadAssetsFile(assetsPath, true);
        var afile = afi.file;
        am.LoadClassDatabaseFromPackage(afile.Metadata.UnityVersion);

        // --- DEBUG: environment sanity ---
        Console.WriteLine($"Unity version: {afile.Metadata.UnityVersion}");
        Console.WriteLine($"Managed dir: {managedDir}");
        Console.WriteLine($"  - Assembly-CSharp.dll: {File.Exists(Path.Combine(managedDir, "Assembly-CSharp.dll"))}");
        Console.WriteLine($"  - tk2dRuntime.dll:     {File.Exists(Path.Combine(managedDir, "tk2dRuntime.dll"))}");

        // Print which Mono generator is actually set
        var monoGen = typeof(AssetsManager).GetProperty("MonoTempGenerator")?.GetValue(am);
        Console.WriteLine("MonoTempGenerator: " + (monoGen?.GetType().FullName ?? "<null>"));

        // How many MonoBehaviours does this file have?
        int mbTotal = 0;
        foreach (var _ in afile.GetAssetsOfType(AssetClassID.MonoBehaviour)) mbTotal++;
        Console.WriteLine($"MonoBehaviours in {Path.GetFileName(assetsPath)}: {mbTotal}");
        // ---------------------------------

        // // --- DEBUG: enumerate MonoBehaviours with class-name resolution ---
        // int mb_idx = 0, resolved = 0, unresolved = 0;
        // Console.WriteLine("=== MonoBehaviours in " + Path.GetFileName(assetsPath) + " ===");
        // foreach (var mb in afile.GetAssetsOfType(AssetClassID.MonoBehaviour))
        // {
        //     string status, className = "";
        //     try
        //     {
        //         var bf = am.GetBaseField(afi, mb);
        //         var scriptField = bf["m_Script"]; // PPtr<MonoScript>
        //         try
        //         {
        //             var scriptExt = am.GetExtAsset(afi, scriptField);
        //             var scriptBF = scriptExt.baseField;
        //             var nspace = scriptBF["m_Namespace"].AsString;
        //             var cname = scriptBF["m_ClassName"].AsString;
        //             className = string.IsNullOrEmpty(nspace) ? cname : ($"{nspace}.{cname}");
        //             status = "RESOLVED";
        //             resolved++;
        //         }
        //         catch
        //         {
        //             status = "UNRESOLVED_SCRIPT"; // m_Script couldn't be resolved across loaded files
        //             unresolved++;
        //         }

        //         var mName = bf["m_Name"].AsString;
        //         if (status != "UNRESOLVED_SCRIPT")
        //         {
        //             Console.WriteLine($"[{mb_idx}] {status}  class:{className}  name:{mName}");
        //         }
        //     }
        //     catch (Exception e)
        //     {
        //         Console.WriteLine($"[{mb_idx}] ERROR reading MB: {e.GetType().Name}: {e.Message}");
        //     }
        //     mb_idx++;
        // }
        // Console.WriteLine($"Resolved:{resolved}  Unresolved:{unresolved}");
        // Console.WriteLine("=========================================");
        // // ------------------------------------------------------------------

        // // --- DEBUG: heuristic candidates that have tk2d-like fields ---
        // Console.WriteLine("=== Heuristic candidates (have spriteDefinitions/textures fields) ===");
        // int cand = 0;
        // foreach (var mb in afile.GetAssetsOfType(AssetClassID.MonoBehaviour))
        // {
        //     try
        //     {
        //         var bf = am.GetBaseField(afi, mb);
        //         bool hasDefs = false, hasTex = false;
        //         try { var _ = bf["spriteDefinitions.Array"].AsArray.size; hasDefs = true; } catch { }
        //         try { var _ = bf["textures.Array"].AsArray.size; hasTex = true; } catch { }
        //         if (hasDefs || hasTex)
        //         {
        //             var name = bf["m_Name"].AsString ?? "(no name)";
        //             Console.WriteLine($"[{cand}] name:{name}  hasDefs:{hasDefs}  hasTex:{hasTex}");
        //             cand++;
        //         }
        //     }
        //     catch { /* ignore */ }
        // }
        // Console.WriteLine("=========================================");
        // // ------------------------------------------------------------------



        // {
        //     int idx = 0;
        //     Console.WriteLine("=== tk2dSpriteCollectionData found in " + Path.GetFileName(assetsPath) + " ===");
        //     foreach (var mb in afile.GetAssetsOfType(AssetClassID.MonoBehaviour))
        //     {
        //         var cls = TryGetMonoBehaviourClassName(am, afi, mb);
        //         if (string.Equals(cls, "tk2dSpriteCollectionData", StringComparison.Ordinal))
        //         {
        //             var bf = am.GetBaseField(afi, mb);
        //             var colName = bf["m_Name"].AsString ?? "(no name)";
        //             int spriteCount = 0, texCount = 0;
        //             try { spriteCount = bf["spriteDefinitions.Array"].AsArray.size; } catch { }
        //             try { texCount = bf["textures.Array"].AsArray.size; } catch { }
        //             Console.WriteLine($"[{idx}] {colName}  sprites:{spriteCount}  textures:{texCount}");
        //             idx++;
        //         }
        //     }
        //     Console.WriteLine("=========================================");
        // }

        // {
        //     int idx = 0;
        //     Console.WriteLine("=== tk2dSpriteCollectionData details in " + Path.GetFileName(assetsPath) + " ===");
        //     foreach (var mb in afile.GetAssetsOfType(AssetClassID.MonoBehaviour))
        //     {
        //         var cls = TryGetMonoBehaviourClassName(am, afi, mb);
        //         if (!string.Equals(cls, "tk2dSpriteCollectionData", StringComparison.Ordinal))
        //             continue;

        //         var bf = am.GetBaseField(afi, mb);
        //         var colName = bf["m_Name"].AsString ?? "";

        //         int spriteCount = 0, texCount = 0;
        //         try { spriteCount = bf["spriteDefinitions.Array"].AsArray.size; } catch { }
        //         try { texCount = bf["textures.Array"].AsArray.size; } catch { }

        //         string tex0Name = "";
        //         int texW = 0, texH = 0;
        //         try
        //         {
        //             var texArr = bf["textures.Array"];
        //             if (texArr.AsArray.size > 0)
        //             {
        //                 var texExt = am.GetExtAsset(afi, texArr[0]);
        //                 var tbf = texExt.baseField;
        //                 tex0Name = tbf["m_Name"].AsString ?? "";
        //                 try { texW = tbf["m_Width"].AsInt; texH = tbf["m_Height"].AsInt; } catch { }
        //             }
        //         }
        //         catch { }

        //         // Grab up to 5 sprite names as a fingerprint
        //         var samples = new List<string>();
        //         try
        //         {
        //             var defsField = bf["spriteDefinitions.Array"];
        //             int take = Math.Min(defsField.AsArray.size, 5);
        //             for (int i = 0; i < take; i++)
        //             {
        //                 var dn = defsField[i]["name"].AsString;
        //                 if (!string.IsNullOrEmpty(dn)) samples.Add(dn);
        //             }
        //         }
        //         catch { }

        //         Console.WriteLine($"[{idx}] coll:'{colName}' sprites:{spriteCount} textures:{texCount} " +
        //                         $"tex0:'{tex0Name}' {texW}x{texH} " +
        //                         $"samples:[{string.Join(", ", samples)}]");
        //         idx++;
        //     }
        //     Console.WriteLine("==============================================================");
        // }
        // // ----------------------------------------------------------------

        // Find all tk2dSpriteCollectionData (no name filter) and pick by index.
        var collections = new List<(AssetsFileInstance afi, AssetFileInfo info)>();
        foreach (var mbInfo in afile.GetAssetsOfType(AssetClassID.MonoBehaviour))
        {
            var cls = TryGetMonoBehaviourClassName(am, afi, mbInfo);
            if (string.Equals(cls, "tk2dSpriteCollectionData", StringComparison.Ordinal))
                collections.Add((afi, mbInfo));
        }

        if (collections.Count == 0)
        {
            Console.Error.WriteLine("No tk2dSpriteCollectionData found in this assets file.");
            return 5;
        }

        // Hardcode the desired collection index from your listing:
        int collectionIndex = 0; // <-- Knight collection

        if (collectionIndex < 0 || collectionIndex >= collections.Count)
        {
            Console.Error.WriteLine($"collectionIndex {collectionIndex} is out of range (0..{collections.Count - 1}).");
            return 5;
        }

        var pickedAfi = collections[collectionIndex].afi;
        var pickedInfo = collections[collectionIndex].info;
        var pickedField = am.GetBaseField(pickedAfi, pickedInfo);
        Console.WriteLine($"Exporting collection index {collectionIndex}");

        // Atlas dims from first Texture2D in 'textures'
        int atlasW = 0, atlasH = 0;
        try
        {
            var textures = pickedField["textures.Array"];
            if (textures.AsArray.size > 0)
            {
                var texExt = am.GetExtAsset(pickedAfi, textures[0]);
                var tbf = texExt.baseField;
                atlasW = tbf["m_Width"].AsInt;
                atlasH = tbf["m_Height"].AsInt;
            }
        }
        catch { /* ok */ }

        var root = new ExportRoot { AtlasWidth = atlasW, AtlasHeight = atlasH };

        var defsField = pickedField["spriteDefinitions.Array"];
        int defCount = defsField.AsArray.size;

        for (int i = 0; i < defCount; i++)
        {
            var def = defsField[i];
            var name = def["name"].AsString ?? $"sprite_{i}";

            // 1) Rotation / flip (tk2d FlipMode)
            int flipInt = 0;
            try { flipInt = def["flipped"].AsInt; } catch { }
            string flipStr = flipInt switch { 1 => "Tk2d", 2 => "TPackerCW", _ => "None" };

            // 2) PixelRect (prefer precise region; else UVs)
            int px = 0, py = 0, pw = 0, ph = 0;
            bool usedRegion = false;
            try
            {
                if (def["extractRegion"].AsBool)
                {
                    px = def["regionX"].AsInt;
                    py = def["regionY"].AsInt;
                    pw = def["regionW"].AsInt;
                    ph = def["regionH"].AsInt;
                    usedRegion = true;
                }
            }
            catch { /* fall through */ }

            if (!usedRegion && atlasW > 0 && atlasH > 0)
            {
                float uMin = 1e9f, vMin = 1e9f, uMax = -1e9f, vMax = -1e9f;
                try
                {
                    var uvs = def["uvs.Array"];
                    int uvCount = uvs.AsArray.size;
                    for (int j = 0; j < uvCount; j++)
                    {
                        var u = uvs[j]["x"].AsFloat;
                        var v = uvs[j]["y"].AsFloat;
                        if (u < uMin) uMin = u; if (v < vMin) vMin = v;
                        if (u > uMax) uMax = u; if (v > vMax) vMax = v;
                    }
                }
                catch { uMin = vMin = uMax = vMax = 0; }

                px = (int)MathF.Round(uMin * atlasW);
                pw = (int)MathF.Round((uMax - uMin) * atlasW);
                py = (int)MathF.Round(vMin * atlasH);
                ph = (int)MathF.Round((vMax - vMin) * atlasH);
            }

            // 3) Pivot (keep bounds.center in world units; don’t serialize bounds)
            float pivotX = 0f, pivotY = 0f;
            float bx = 0, by = 0, ex = 0, ey = 0, ubx = 0, uby = 0, uex = 0, uey = 0;
            bool haveBounds = false, haveUntrimmed = false;
            try
            {
                var bd = def["boundsData.Array"];
                bx = bd[0]["x"].AsFloat; by = bd[0]["y"].AsFloat;
                ex = bd[1]["x"].AsFloat; ey = bd[1]["y"].AsFloat;
                pivotX = bx; pivotY = by;
                haveBounds = true;
            }
            catch { /* ok */ }

            try
            {
                var ubd = def["untrimmedBoundsData.Array"];
                ubx = ubd[0]["x"].AsFloat; uby = ubd[0]["y"].AsFloat;
                uex = ubd[1]["x"].AsFloat; uey = ubd[1]["y"].AsFloat;
                haveUntrimmed = true;
            }
            catch { /* ok */ }

            // 4) Compute TexturePacker-style trim offsets in pixels
            //    We use texelSize (world units per pixel) internally but DO NOT serialize it.
            float texelX = 1f, texelY = 1f;
            try
            {
                texelX = pickedField["texelSize"]["x"].AsFloat;
                texelY = pickedField["texelSize"]["y"].AsFloat;
            }
            catch { /* default 1 */ }

            // Source (untrimmed) size in pixels
            int srcW = 0, srcH = 0, offX = 0, offY = 0;
            if (haveUntrimmed)
            {
                // width/height in world units: 2*extents; convert to pixels by dividing by texel
                srcW = (int)MathF.Round((uex * 2f) / texelX);
                srcH = (int)MathF.Round((uey * 2f) / texelY);

                if (haveBounds)
                {
                    // bottom-left in world units
                    var untrimBLx = ubx - uex;
                    var untrimBLy = uby - uey;
                    var trimBLx = bx - ex;
                    var trimBLy = by - ey;

                    // offset in pixels from untrimmed bottom-left to trimmed bottom-left
                    offX = (int)MathF.Round((trimBLx - untrimBLx) / texelX);
                    offY = (int)MathF.Round((trimBLy - untrimBLy) / texelY);

                    // clamp to sane range
                    if (offX < 0) offX = 0;
                    if (offY < 0) offY = 0;
                    if (pw > 0 && srcW > 0 && offX + pw > srcW) offX = Math.Max(0, srcW - pw);
                    if (ph > 0 && srcH > 0 && offY + ph > srcH) offY = Math.Max(0, srcH - ph);
                }
            }

            // 5) Emit
            root.Sprites.Add(new ExportSprite
            {
                Name = name,
                PixelRect = new PixelRect { X = px, Y = py, W = pw, H = ph },
                Pivot = new[] { pivotX, pivotY },
                Flip = flipStr,
                SourceSize = new Size2D { W = srcW, H = srcH },
                SpriteSourceOffset = new Offset2D { X = offX, Y = offY }
            });
        }

        var json = JsonSerializer.Serialize(root, new JsonSerializerOptions
        {
            WriteIndented = true,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
            IncludeFields = true
        });
        File.WriteAllText(outJsonPath, json);
        Console.WriteLine($"Exported {root.Sprites.Count} sprites → {outJsonPath}");
        return 0;
    }
}
