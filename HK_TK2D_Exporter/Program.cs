using System.Text.Json;
using System.Text.Json.Serialization;
using AssetsTools.NET;
using AssetsTools.NET.Extra;

public sealed class ExportRoot
{
    public int AtlasWidth { get; set; }
    public int AtlasHeight { get; set; }
    public string CollectionName { get; set; } = "";
    public float[] TexelSize { get; set; } = new float[2];
    public List<ExportSprite> Sprites { get; set; } = new();
}
public sealed class ExportSprite
{
    public string Name { get; set; } = "";
    public UVRect UVRect { get; set; } = new();
    public PixelRect PixelRect { get; set; } = new();
    public BoundsData Bounds { get; set; } = new();
    public BoundsData UntrimmedBounds { get; set; } = new();
    public float[] Pivot { get; set; } = new float[2];
    public int[]? Indices { get; set; }
}
public sealed class UVRect { public float UMin, VMin, UMax, VMax; }
public sealed class PixelRect { public int X, Y, W, H; }
public sealed class BoundsData { public float[] Center = new float[3]; public float[] Extents = new float[3]; }

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
        int collectionIndex = 0; // <-- the Knight collection you identified

        if (collectionIndex < 0 || collectionIndex >= collections.Count)
        {
            Console.Error.WriteLine($"collectionIndex {collectionIndex} is out of range (0..{collections.Count - 1}).");
            return 5;
        }

        var pickedAfi = collections[collectionIndex].afi;
        var pickedInfo = collections[collectionIndex].info;
        var pickedField = am.GetBaseField(pickedAfi, pickedInfo);
        var collectionNm = pickedField["m_Name"].AsString ?? "tk2dCollection";
        Console.WriteLine($"Exporting collection index {collectionIndex} → '{collectionNm}'");


        // Atlas dims from first Texture2D in 'textures'
        // Atlas dims from first Texture2D in 'textures'
        int atlasW = 0, atlasH = 0;
        try
        {
            var textures = pickedField["textures.Array"];
            if (textures.AsArray.size > 0)
            {
                var texPtrField = textures[0];
                var texExt = am.GetExtAsset(pickedAfi, texPtrField); // <-- use pickedAfi
                var tbf = texExt.baseField;
                atlasW = tbf["m_Width"].AsInt;
                atlasH = tbf["m_Height"].AsInt;
            }
        }
        catch { /* will still export UVs */ }


        float texelX = 1f, texelY = 1f;
        try { texelX = pickedField["texelSize"]["x"].AsFloat; texelY = pickedField["texelSize"]["y"].AsFloat; } catch { }

        var root = new ExportRoot
        {
            AtlasWidth = atlasW,
            AtlasHeight = atlasH,
            CollectionName = collectionNm,
            TexelSize = new[] { texelX, texelY }
        };

        var defs = pickedField["spriteDefinitions.Array"];
        var defCount = defs.AsArray.size;

        for (int i = 0; i < defCount; i++)
        {
            var def = defs[i];
            var name = def["name"].AsString ?? $"sprite_{i}";

            float uMin = 1e9f, vMin = 1e9f, uMax = -1e9f, vMax = -1e9f;
            try
            {
                var uvs = def["uvs.Array"];
                var n = uvs.AsArray.size;
                for (int j = 0; j < n; j++)
                {
                    var u = uvs[j]["x"].AsFloat;
                    var v = uvs[j]["y"].AsFloat;
                    if (u < uMin) uMin = u; if (v < vMin) vMin = v;
                    if (u > uMax) uMax = u; if (v > vMax) vMax = v;
                }
            }
            catch { uMin = vMin = uMax = vMax = 0; }

            int px = 0, py = 0, pw = 0, ph = 0;
            if (atlasW > 0 && atlasH > 0)
            {
                px = (int)MathF.Round(uMin * atlasW);
                pw = (int)MathF.Round((uMax - uMin) * atlasW);
                py = (int)MathF.Round(vMin * atlasH);
                ph = (int)MathF.Round((vMax - vMin) * atlasH);
            }

            var b = new BoundsData();
            var ub = new BoundsData();
            try
            {
                var bd = def["boundsData.Array"];
                b.Center[0] = bd[0]["x"].AsFloat; b.Center[1] = bd[0]["y"].AsFloat; b.Center[2] = bd[0]["z"].AsFloat;
                b.Extents[0] = bd[1]["x"].AsFloat; b.Extents[1] = bd[1]["y"].AsFloat; b.Extents[2] = bd[1]["z"].AsFloat;

                var ubd = def["untrimmedBoundsData.Array"];
                ub.Center[0] = ubd[0]["x"].AsFloat; ub.Center[1] = ubd[0]["y"].AsFloat; ub.Center[2] = ubd[0]["z"].AsFloat;
                ub.Extents[0] = ubd[1]["x"].AsFloat; ub.Extents[1] = ubd[1]["y"].AsFloat; ub.Extents[2] = ubd[1]["z"].AsFloat;
            }
            catch { /* older defs may lack untrimmed */ }

            int[]? indices = null;
            try
            {
                var inds = def["indices.Array"];
                var n = inds.AsArray.size;
                indices = new int[n];
                for (int j = 0; j < n; j++) indices[j] = inds[j].AsInt;
            }
            catch { }

            var pivotX = b.Center[0];
            var pivotY = b.Center[1];

            root.Sprites.Add(new ExportSprite
            {
                Name = name,
                UVRect = new UVRect { UMin = uMin, VMin = vMin, UMax = uMax, VMax = vMax },
                PixelRect = new PixelRect { X = px, Y = py, W = pw, H = ph },
                Bounds = b,
                UntrimmedBounds = ub,
                Pivot = new[] { pivotX, pivotY },
                Indices = indices
            });
        }

        var json = JsonSerializer.Serialize(root, new JsonSerializerOptions
        {
            WriteIndented = true,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
            IncludeFields = true
        });
        File.WriteAllText(outJsonPath, json);
        Console.WriteLine($"Exported {root.Sprites.Count} sprites from '{root.CollectionName}'");
        Console.WriteLine($"→ {outJsonPath}");
        return 0;
    }
}
