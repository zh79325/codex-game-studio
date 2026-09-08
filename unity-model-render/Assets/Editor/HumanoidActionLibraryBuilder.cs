using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using UnityEditor;
using UnityEditor.Animations;
using UnityEngine;

namespace ModelPreview.Editor
{
    public static class HumanoidActionLibraryBuilder
    {
        private const string SourceDirectoryEnvironment = "CODEX_GAME_ANIMATIONS_DIR";
        private const string AnimationDirectory = "Assets/ModelPreview/AnimationSources";
        private const string ControllerDirectory = "Assets/ModelPreview/Resources/ModelPreview";
        private const string ControllerPath = ControllerDirectory + "/HumanoidActionLibrary.controller";

        private static readonly ActionAsset[] Actions =
        {
            new ActionAsset("X Bot@Breathing Idle.fbx", "BreathingIdle.fbx", "Idle", true, false),
            new ActionAsset("X Bot@Standard Walk.fbx", "StandardWalk.fbx", "Walk", true, false),
            new ActionAsset("X Bot@Run.fbx", "Run.fbx", "Run", true, false),
            new ActionAsset("X Bot@Punching Bag.fbx", "PunchingBag.fbx", "Punch", false, true),
            new ActionAsset("X Bot@Mma Kick.fbx", "MmaKick.fbx", "Kick", false, true),
            new ActionAsset(
                "X Bot@Heavy Weapon Swing.fbx",
                "HeavyWeaponSwing.fbx",
                "HeavyWeaponSwing",
                false,
                true)
        };

        [MenuItem("Tools/Model Preview/Build Humanoid Action Library")]
        public static void Build()
        {
            var sourceDirectory = ResolveSourceDirectory();
            if (!Directory.Exists(sourceDirectory))
            {
                throw new DirectoryNotFoundException(
                    "Mixamo action directory does not exist: " + sourceDirectory
                    + ". Set " + SourceDirectoryEnvironment + " to override it.");
            }

            EnsureAssetDirectory(AnimationDirectory);
            EnsureAssetDirectory(ControllerDirectory);
            // Animation-only Mixamo FBXs can expose their first action frame as
            // the source skeleton pose. Build one Avatar from the neutral idle
            // asset and reuse it so every clip is humanized against one basis.
            var sourceAvatar = ImportAction(sourceDirectory, Actions[0], null);
            foreach (var action in Actions.Skip(1))
            {
                ImportAction(sourceDirectory, action, sourceAvatar);
            }

            AssetDatabase.Refresh(ImportAssetOptions.ForceSynchronousImport);
            var controller = BuildController();
            ValidateLibrary(controller);
            AssetDatabase.SaveAssets();
            Debug.Log(
                "[HumanoidActionLibrary] Ready; source=" + sourceDirectory
                + "; controller=" + ControllerPath
                + "; actions=" + Actions.Length);
        }

        [MenuItem("Tools/Model Preview/Validate Humanoid Action Library")]
        public static void Validate()
        {
            var controller = AssetDatabase.LoadAssetAtPath<AnimatorController>(ControllerPath);
            ValidateLibrary(controller);
            Debug.Log("[HumanoidActionLibrary] Validation passed: " + ControllerPath);
        }

        private static string ResolveSourceDirectory()
        {
            var configured = Environment.GetEnvironmentVariable(SourceDirectoryEnvironment);
            if (!string.IsNullOrWhiteSpace(configured))
            {
                return Path.GetFullPath(configured.Trim());
            }
            return Path.GetFullPath(Path.Combine(Application.dataPath, "../../../animations"));
        }

        private static Avatar ImportAction(
            string sourceDirectory,
            ActionAsset action,
            Avatar sourceAvatar)
        {
            var sourcePath = Path.Combine(sourceDirectory, action.SourceName);
            if (!File.Exists(sourcePath))
            {
                throw new FileNotFoundException("Missing Mixamo action: " + sourcePath, sourcePath);
            }

            var assetPath = AnimationDirectory + "/" + action.AssetName;
            var absoluteAssetPath = Path.GetFullPath(Path.Combine(Application.dataPath, "..", assetPath));
            File.Copy(sourcePath, absoluteAssetPath, true);
            AssetDatabase.ImportAsset(assetPath, ImportAssetOptions.ForceSynchronousImport);

            var importer = AssetImporter.GetAtPath(assetPath) as ModelImporter;
            if (importer == null)
            {
                throw new InvalidOperationException("FBX has no ModelImporter: " + assetPath);
            }

            importer.importAnimation = true;
            importer.animationType = ModelImporterAnimationType.Human;
            importer.materialImportMode = ModelImporterMaterialImportMode.None;
            importer.importCameras = false;
            importer.importLights = false;
            importer.importBlendShapes = false;
            importer.optimizeGameObjects = false;
            if (sourceAvatar == null)
            {
                importer.avatarSetup = ModelImporterAvatarSetup.CreateFromThisModel;
                importer.sourceAvatar = null;
            }
            else
            {
                importer.avatarSetup = ModelImporterAvatarSetup.CopyFromOther;
                importer.sourceAvatar = sourceAvatar;
            }

            var clips = importer.defaultClipAnimations;
            if (clips == null || clips.Length == 0)
            {
                throw new InvalidOperationException("FBX contains no animation take: " + assetPath);
            }
            for (var index = 0; index < clips.Length; index++)
            {
                var clip = clips[index];
                clip.name = index == 0 ? action.StateName : action.StateName + "_" + (index + 1);
                clip.loopTime = action.Loop;
                clip.loopPose = action.Loop;
                clip.lockRootRotation = true;
                clip.lockRootHeightY = true;
                clip.lockRootPositionXZ = true;
                clip.keepOriginalOrientation = true;
                clip.keepOriginalPositionY = false;
                clip.keepOriginalPositionXZ = false;
                clips[index] = clip;
            }
            importer.clipAnimations = clips;
            importer.SaveAndReimport();

            if (sourceAvatar != null)
            {
                return sourceAvatar;
            }

            var avatar = AssetDatabase.LoadAllAssetsAtPath(assetPath).OfType<Avatar>().FirstOrDefault();
            if (avatar == null || !avatar.isValid || !avatar.isHuman)
            {
                throw new InvalidOperationException("Reference Mixamo FBX did not produce a valid Human Avatar: " + assetPath);
            }
            return avatar;
        }

        private static AnimatorController BuildController()
        {
            var controller = AssetDatabase.LoadAssetAtPath<AnimatorController>(ControllerPath);
            if (controller == null)
            {
                controller = AnimatorController.CreateAnimatorControllerAtPath(ControllerPath);
            }

            var stateMachine = controller.layers[0].stateMachine;
            foreach (var childState in stateMachine.states.ToArray())
            {
                stateMachine.RemoveState(childState.state);
            }

            AnimatorState idleState = null;
            var actionStates = new List<AnimatorState>();
            foreach (var action in Actions)
            {
                var assetPath = AnimationDirectory + "/" + action.AssetName;
                var clip = AssetDatabase.LoadAllAssetsAtPath(assetPath)
                    .OfType<AnimationClip>()
                    .FirstOrDefault(candidate => candidate.name == action.StateName);
                if (clip == null || clip.legacy || !clip.humanMotion)
                {
                    throw new InvalidOperationException(
                        "Expected a non-Legacy Humanoid clip named " + action.StateName + " in " + assetPath);
                }

                var state = stateMachine.AddState(action.StateName);
                state.motion = clip;
                state.writeDefaultValues = false;
                if (action.StateName == "Idle")
                {
                    idleState = state;
                }
                if (action.ReturnToIdle)
                {
                    actionStates.Add(state);
                }
            }

            if (idleState == null)
            {
                throw new InvalidOperationException("The action library requires an Idle state.");
            }
            stateMachine.defaultState = idleState;
            foreach (var state in actionStates)
            {
                var transition = state.AddTransition(idleState);
                transition.hasExitTime = true;
                transition.exitTime = 1f;
                transition.hasFixedDuration = true;
                transition.duration = 0.08f;
                transition.interruptionSource = TransitionInterruptionSource.None;
            }

            EditorUtility.SetDirty(controller);
            return controller;
        }

        private static void ValidateLibrary(AnimatorController controller)
        {
            if (controller == null)
            {
                throw new InvalidOperationException("Missing action controller: " + ControllerPath);
            }

            var states = controller.layers[0].stateMachine.states
                .Select(child => child.state)
                .ToDictionary(state => state.name, StringComparer.Ordinal);
            foreach (var action in Actions)
            {
                if (!states.TryGetValue(action.StateName, out var state)
                    || state.motion is not AnimationClip clip
                    || clip.legacy
                    || !clip.humanMotion)
                {
                    throw new InvalidOperationException(
                        "Invalid Humanoid state " + action.StateName + " in " + ControllerPath);
                }
            }
        }

        private static void EnsureAssetDirectory(string path)
        {
            var segments = path.Split('/');
            var current = segments[0];
            for (var index = 1; index < segments.Length; index++)
            {
                var next = current + "/" + segments[index];
                if (!AssetDatabase.IsValidFolder(next))
                {
                    AssetDatabase.CreateFolder(current, segments[index]);
                }
                current = next;
            }
        }

        private readonly struct ActionAsset
        {
            public ActionAsset(
                string sourceName,
                string assetName,
                string stateName,
                bool loop,
                bool returnToIdle)
            {
                SourceName = sourceName;
                AssetName = assetName;
                StateName = stateName;
                Loop = loop;
                ReturnToIdle = returnToIdle;
            }

            public string SourceName { get; }
            public string AssetName { get; }
            public string StateName { get; }
            public bool Loop { get; }
            public bool ReturnToIdle { get; }
        }
    }
}
