use codex_api::ImageBackground;
use codex_api::ImageEditRequest;
use codex_api::ImageGenerationRequest;
use codex_api::ImageQuality;
use codex_api::ImageUrl;
use codex_extension_api::ToolOutput;
use codex_extension_api::ToolPayload;
use codex_extension_api::ToolSpec;
use codex_protocol::ResponseItemId;
use codex_protocol::models::ContentItem;
use codex_protocol::models::DEFAULT_IMAGE_DETAIL;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_tools::ResponsesApiNamespaceTool;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;

use super::GeneratedImageOutput;
use super::ImageGenerationTurnGate;
use super::ImageRequest;
use super::ImagegenArgs;
use super::apply_required_reference_paths;
use super::image_executor_tool_spec;
use super::imagegen_tool_spec;
use super::request_for_call_args;
use super::validate_executor_args;
use crate::IMAGE_GEN_NAMESPACE;
use crate::IMAGEGEN_TOOL_NAME;
use crate::artifact::image_generation_artifact_path;
use crate::artifact::image_generation_output_hint;

const RESULT: &str = "cG5n";

#[test]
fn artifact_path_sanitizes_session_and_call_ids() {
    let save_root = AbsolutePathBuf::current_dir().expect("current directory should be absolute");

    assert_eq!(
        image_generation_artifact_path(&save_root, "../session", "../call"),
        save_root
            .join("generated_images")
            .join("___session")
            .join("___call.png")
    );
}

#[test]
fn uses_reserved_image_gen_namespace() {
    let ToolSpec::Namespace(spec) = imagegen_tool_spec() else {
        panic!("imagegen should advertise a namespace tool");
    };
    assert_eq!(spec.name, IMAGE_GEN_NAMESPACE);
    let ResponsesApiNamespaceTool::Function(function) = &spec.tools[0] else {
        panic!("imagegen should advertise a function tool");
    };
    assert_eq!(function.name, IMAGEGEN_TOOL_NAME);
}

#[test]
fn exposes_stage_executor_as_a_plain_function_tool() {
    let ToolSpec::Function(spec) = image_executor_tool_spec("image_t2i") else {
        panic!("stage executor should advertise a function tool");
    };
    assert_eq!(spec.name, "image_t2i");
}

#[test]
fn image_tools_share_one_call_per_turn() {
    let gate = ImageGenerationTurnGate::default();

    assert!(gate.consume("turn-1"));
    assert!(!gate.consume("turn-1"));
    assert!(gate.consume("turn-2"));
}

#[test]
fn logical_scope_survives_contract_retry_and_resets_for_new_task() {
    let gate = ImageGenerationTurnGate::default();

    gate.begin_scope("task-1");
    assert!(gate.consume("provider-turn-1"));
    gate.begin_scope("task-1");
    assert!(!gate.consume("provider-turn-2"));

    gate.begin_scope("task-2");
    assert!(gate.consume("provider-turn-3"));
}

#[test]
fn recovered_artifact_preconsumes_generation_scope() {
    let gate = ImageGenerationTurnGate::default();

    gate.begin_scope_with_consumed("task-recovery", true);
    assert!(!gate.consume("provider-turn-1"));

    gate.begin_scope_with_consumed("task-new", false);
    assert!(gate.consume("provider-turn-2"));
}

#[test]
fn stage_executors_enforce_their_reference_contract() {
    let t2i_with_reference = ImagegenArgs {
        prompt: "render".to_string(),
        referenced_image_paths: Some(vec![
            "/tmp/reference.png"
                .try_into()
                .expect("test path should be absolute"),
        ]),
        num_last_images_to_include: None,
    };
    assert!(validate_executor_args(Some("image_t2i"), &t2i_with_reference).is_err());

    let i2i_without_references = ImagegenArgs {
        prompt: "views".to_string(),
        referenced_image_paths: None,
        num_last_images_to_include: None,
    };
    assert!(validate_executor_args(Some("image_i2i"), &i2i_without_references).is_err());

    let i2i_with_one_reference = ImagegenArgs {
        prompt: "render revision".to_string(),
        referenced_image_paths: None,
        num_last_images_to_include: Some(1),
    };
    assert!(validate_executor_args(Some("image_i2i"), &i2i_with_one_reference).is_ok());
}

#[test]
fn required_references_are_prioritized_and_added_once() {
    let render = AbsolutePathBuf::from_absolute_path_checked("/tmp/render.png")
        .expect("test path should be absolute");
    let template = AbsolutePathBuf::from_absolute_path_checked("/tmp/t-pose-template.png")
        .expect("test path should be absolute");
    let candidate = AbsolutePathBuf::from_absolute_path_checked("/tmp/candidate.png")
        .expect("test path should be absolute");
    let mut args = ImagegenArgs {
        prompt: "views".to_string(),
        referenced_image_paths: Some(vec![candidate.clone(), render.clone()]),
        num_last_images_to_include: None,
    };

    apply_required_reference_paths(&mut args, &[template.clone(), render.clone()])
        .expect("required references should fit");

    assert_eq!(
        args.referenced_image_paths,
        Some(vec![template, render, candidate])
    );
}

#[tokio::test]
async fn omitted_references_generate_with_fixed_defaults() {
    assert_eq!(
        request_for_call_args(
            &ImagegenArgs {
                prompt: "paint a moonlit lake".to_string(),
                referenced_image_paths: None,
                num_last_images_to_include: None,
            },
            &[],
            &[],
        )
        .await
        .expect("generation request should build"),
        ImageRequest::Generate(ImageGenerationRequest {
            prompt: "paint a moonlit lake".to_string(),
            background: Some(ImageBackground::Auto),
            model: "gpt-image-2".to_string(),
            n: None,
            quality: Some(ImageQuality::Auto),
            size: Some("auto".to_string()),
        })
    );
}

#[tokio::test]
async fn recent_image_fallback_selects_newest_images_in_chronological_order() {
    let history = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![
                input_image("user-1"),
                input_image("user-2"),
                ContentItem::InputText {
                    text: "edit these".to_string(),
                },
            ],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::FunctionCall {
            id: None,
            name: "mcp_image".to_string(),
            namespace: None,
            arguments: "{}".to_string(),
            call_id: "mcp-call".to_string(),
            encrypted_function_args: None,
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::FunctionCallOutput {
            id: None,
            call_id: Some("mcp-call".to_string()),
            name: None,
            namespace: None,
            output: image_output("mcp"),
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::CustomToolCall {
            id: None,
            status: Some("completed".to_string()),
            call_id: "code-mode-call".to_string(),
            name: "exec".to_string(),
            namespace: None,
            input: String::new(),
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::CustomToolCallOutput {
            id: None,
            call_id: "code-mode-call".to_string(),
            name: Some("exec".to_string()),
            output: image_output("code-mode"),
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::ImageGenerationCall {
            id: Some(ResponseItemId::with_suffix("ig", "generated-call")),
            status: "completed".to_string(),
            revised_prompt: None,
            result: "generated".to_string(),
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::FunctionCallOutput {
            id: None,
            call_id: None,
            name: Some("notifications".to_string()),
            namespace: Some("slack".to_string()),
            output: image_output("standalone"),
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    assert_eq!(
        request_for_call_args(
            &ImagegenArgs {
                prompt: "change the lighting".to_string(),
                referenced_image_paths: None,
                num_last_images_to_include: Some(5),
            },
            &history,
            &[],
        )
        .await
        .expect("history-backed edit request should build"),
        ImageRequest::Edit(expected_edit_request(
            "change the lighting",
            &["user-2", "mcp", "code-mode", "generated", "standalone"],
        ))
    );
}

#[tokio::test]
async fn conflicting_image_selectors_return_tool_error() {
    let error = request_for_call_args(
        &ImagegenArgs {
            prompt: "change the lighting".to_string(),
            referenced_image_paths: Some(vec![
                "/tmp/image.png"
                    .try_into()
                    .expect("test path should be absolute"),
            ]),
            num_last_images_to_include: Some(1),
        },
        &[],
        &[],
    )
    .await
    .expect_err("conflicting selectors should fail");

    assert_eq!(
        error.to_string(),
        "provide only one of `referenced_image_paths` or `num_last_images_to_include`"
    );
}

#[tokio::test]
async fn too_many_referenced_image_paths_return_tool_error() {
    let error = request_for_call_args(
        &ImagegenArgs {
            prompt: "change the lighting".to_string(),
            referenced_image_paths: Some(
                (0..6)
                    .map(|index| {
                        format!("/tmp/image-{index}.png")
                            .try_into()
                            .expect("test path should be absolute")
                    })
                    .collect(),
            ),
            num_last_images_to_include: None,
        },
        &[],
        &[],
    )
    .await
    .expect_err("too many paths should fail before reading files");

    assert_eq!(
        error.to_string(),
        "`referenced_image_paths` must contain at most 5 paths"
    );
}

#[tokio::test]
async fn recent_image_fallback_requires_requested_count() {
    let error = request_for_call_args(
        &ImagegenArgs {
            prompt: "change the lighting".to_string(),
            referenced_image_paths: None,
            num_last_images_to_include: Some(2),
        },
        &[ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![input_image("only-image")],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }],
        &[],
    )
    .await
    .expect_err("history-backed edit should require the requested image count");

    assert_eq!(
        error.to_string(),
        "requested the last 2 conversation images, but only 1 were available"
    );
}

#[test]
fn generated_output_returns_image_input_and_output_hint() {
    let output_hint =
        image_generation_output_hint("/tmp", "/tmp/call-1.png").expect("hint should fit");
    let output = GeneratedImageOutput {
        result: RESULT.to_string(),
        output_hint: Some(output_hint.clone()),
    };

    let ResponseInputItem::FunctionCallOutput {
        output: response_output,
        ..
    } = output.to_response_item("call-1", &function_payload())
    else {
        panic!("imagegen should return function tool output");
    };
    let FunctionCallOutputBody::ContentItems(content_items) = response_output.body else {
        panic!("imagegen output should contain generated image bytes");
    };
    assert_eq!(
        content_items,
        vec![
            FunctionCallOutputContentItem::InputImage {
                image_url: format!("data:image/png;base64,{RESULT}"),
                detail: Some(DEFAULT_IMAGE_DETAIL),
            },
            FunctionCallOutputContentItem::InputText { text: output_hint },
        ]
    );
}

#[test]
fn generated_output_returns_generated_image_helper_input_in_code_mode() {
    let output = GeneratedImageOutput {
        result: RESULT.to_string(),
        output_hint: Some("generated image save hint".to_string()),
    };

    assert_eq!(
        output.code_mode_result(&function_payload()),
        serde_json::json!({
            "image_url": format!("data:image/png;base64,{RESULT}"),
            "output_hint": "generated image save hint",
        })
    );
}

#[test]
fn generated_output_omits_oversized_output_hint() {
    let long_path = "x".repeat(1024);
    let output = GeneratedImageOutput {
        result: RESULT.to_string(),
        output_hint: image_generation_output_hint("/tmp", long_path),
    };

    let ResponseInputItem::FunctionCallOutput {
        output: response_output,
        ..
    } = output.to_response_item("call-1", &function_payload())
    else {
        panic!("imagegen should return function tool output");
    };
    let FunctionCallOutputBody::ContentItems(content_items) = response_output.body else {
        panic!("imagegen output should contain generated image bytes");
    };
    assert_eq!(
        content_items,
        vec![FunctionCallOutputContentItem::InputImage {
            image_url: format!("data:image/png;base64,{RESULT}"),
            detail: Some(DEFAULT_IMAGE_DETAIL),
        }]
    );
}

fn input_image(image: &str) -> ContentItem {
    ContentItem::InputImage {
        image_url: format!("data:image/png;base64,{image}"),
        detail: None,
    }
}

fn image_output(image: &str) -> FunctionCallOutputPayload {
    FunctionCallOutputPayload::from_content_items(vec![FunctionCallOutputContentItem::InputImage {
        image_url: format!("data:image/png;base64,{image}"),
        detail: None,
    }])
}

fn expected_edit_request(prompt: &str, images: &[&str]) -> ImageEditRequest {
    ImageEditRequest {
        images: images
            .iter()
            .map(|image| ImageUrl {
                image_url: format!("data:image/png;base64,{image}"),
            })
            .collect(),
        prompt: prompt.to_string(),
        background: Some(ImageBackground::Auto),
        model: "gpt-image-2".to_string(),
        n: None,
        quality: Some(ImageQuality::Auto),
        size: Some("auto".to_string()),
    }
}

fn function_payload() -> ToolPayload {
    ToolPayload::Function {
        arguments: "{}".to_string(),
    }
}
