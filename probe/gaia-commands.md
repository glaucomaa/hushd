# GAIA v3: карта команд MOMENTUM 4

Извлечено из расшифрованной модели устройства `re/device-model/m4.json` проекта
[xndbogdan/sennheiser-control](https://github.com/xndbogdan/sennheiser-control)
(реверс Smart Control 4.9.3, `com.sennheiser.control`). Это **не** наша добыча:
таблица принадлежит автору репозитория и приведена здесь как справочник.

`ID` — базовый id команды; ответ = `ID | 0x0100`, уведомление = `ID | 0x0080`,
ошибка = `ID | 0x0180`.

## Vendor 0x0495 (Sennheiser)

| ID | Команда | Параметры запроса | Ответ | Уведомление | Свойство |
|---|---|---|---|---|---|
| `0x0007` | Config_RegisterNotification_Sennheiser | UINT8 featureID | да | — | `RegisterNotification_Device` |
| `0x0040` | Service_FactoryReset | — | да | — | `Service_FactoryReset` |
| `0x0041` | Development_Primary_Side | — | да | — | `Primary_Side` |
| `0x0200` | Upgrade_EnterDfuMode | — | да | — | `Upgrade_Enable` |
| `0x0201` | Upgrade_SetUpgradeFilesize | UINT32 sizeOfUpgradeFile | да | — | `UpgradeFileSize` |
| `0x0202` | — | — | да | — | `UpgradeProgress` |
| `0x0400` | OnHeadDetection_Mode_Set | UINT8 mode | да | — | `Setting_OnHeadDetection` |
| `0x0401` | OnHeadDetection_Mode_Get | — | да | — | `Setting_OnHeadDetection` |
| `0x0402` | PhysicalDevice_State_Get | — | да | — | `PhysicalDevice_State` |
| `0x0405` | BluetoothCompatibilityMode_Set | UINT8 mode | да | — | `BluetoothCompatibilityMode` |
| `0x0406` | BluetoothCompatibilityMode_Get | — | да | — | `BluetoothCompatibilityMode` |
| `0x0600` | Config_SetTimerConfiguration | UINT8 Timer number, UINT16 Timer value | да | — | `Config_SetTimer` |
| `0x0601` | Config_GetTimerConfiguration | UINT8 Timer number | да | — | `Config_GetTimer` |
| `0x0602` | Battery_ChargingState_Get_TW | — | да | — | `Battery_ChargingStatus` |
| `0x0603` | Battery_GetBatteryLevels | — | да | — | `Battery_Percent` |
| `0x0800` | System_CodecUsed_Get_TW | — | да | — | `Sound_CodecUsed` |
| `0x0801` | AudioPrompt_Mode_Set | UINT8 mode | да | — | `Setting_AudioPromptMode` |
| `0x0802` | AudioPrompt_Mode_Get | — | да | — | `Setting_AudioPromptMode` |
| `0x0805` | Sidetone_SetState | UINT8 sidetone state | да | — | `SidetoneState` |
| `0x0806` | Sidetone_GetState | — | да | — | `SidetoneState` |
| `0x0807` | AudioPrompt_CustomLanguage_Get_TW | — | да | — | `VoicePrompt_Language` |
| `0x0808` | AudioPrompt_GetAvailableAudioPromptsLanguages | — | да | — | `AudioPrompt_GetAvailableAudioPromptsLanguages` |
| `0x0809` | AudioPrompt_GetAudioPromptsLanguageVersions | UINT8 side | да | — | `AudioPrompt_GetAudioPromptsLanguageVersions` |
| `0x080A` | AutoCall_Mode_Set | UINT8 mode | да | — | `Setting_AutoCall` |
| `0x080B` | AutoCall_Mode_Get | — | да | — | `Setting_AutoCall` |
| `0x080C` | System_AutoPause_Set | UINT8 mode | да | — | `Setting_SmartPause` |
| `0x080D` | System_AutoPause_Get | — | да | — | `Setting_SmartPause` |
| `0x0814` | Call_Externalization_Set | UINT8 mode | да | — | `Setting_ComfortCall` |
| `0x0815` | Call_Externalization_Get | — | да | — | `Setting_ComfortCall` |
| `0x0817` | LowLatencyMode_Set | UINT8 LowLatencymode | да | — | `LowLatencyMode` |
| `0x0818` | LowLatencyMode_Get | — | да | — | `LowLatencyMode` |
| `0x1200` | Service_GetHwRevision_TW | — | да | — | `Service_HWRevision` |
| `0x1201` | SERVICE_GetSystemReleaseVersion | — | да | — | `Service_SystemReleaseVersion` |
| `0x1202` | System_Version_Get_TW | — | да | — | `FirmwareVersions` |
| `0x1206` | Versions_GetModelId | — | да | — | `Versions_ModelId` |
| `0x1400` | DeviceList_GetPairedDeviceListSize | — | да | — | `PairedDevicesListSize` |
| `0x1401` | DeviceList_GetDeviceInfo | UINT8 deviceIndex | да | — | `PairedDevicesGetDeviceInfo` |
| `0x1402` | DeviceList_Connect | UINT8 deviceIndex | да | — | `PairedDevicesConnectDevice` |
| `0x1403` | DeviceList_Disconnect | UINT8 deviceIndex | да | — | `PairedDevicesDisconnectDevice` |
| `0x1404` | DeviceList_GetConnectionStatus | UINT8 deviceIndex | да | — | `PairedDevicesGetConnectionStatus` |
| `0x1405` | DeviceList_DeleteEntry | UINT8 deviceIndex | да | — | `PairedDevicesDeleteEntry` |
| `0x1406` | DeviceList_DeletePDL | — | да | — | `PairedDevicesDeleteList` |
| `0x1407` | DeviceList_GetOwnDeviceIndex | — | да | — | `PairedDevicesOwnDeviceIndex` |
| `0x1409` | DeviceList_GetMaxBtConnections | — | да | — | `PairedDevicesMaxBTConnections` |
| `0x1600` | MMI_SetConfig | UINT8 button number, UINT8 pattern, UINT8 function | да | — | `SetMMIConfig` |
| `0x1601` | MMI_GetConfig | UINT8 button number, UINT8 pattern | да | — | `GetMMIConfig` |
| `0x1604` | MMI_SetDefaultConfig | — | да | — | `SetDefaultMMIConfig` |
| `0x1605` | MMI_IsDefaultConfig | — | да | — | `IsDefaultMMIConfig` |
| `0x1606` | System_AutoLock_Set | UINT8 mode | да | — | `Setting_AutoLockMode` |
| `0x1607` | System_AutoLock_Get | — | да | — | `Setting_AutoLockMode` |
| `0x1800` | TransparentHearing_Mode_Set_TW | UINT8 mode | да | — | `TransparentHearingMode` |
| `0x1801` | TransparentHearing_Mode_Get_TW | — | да | — | `TransparentHearingMode` |
| `0x1802` | TransparentHearing_Status_Set_TW | UINT8 status | да | — | `TransparentHearing` |
| `0x1803` | TransparentHearing_Status_Get_TW | — | да | — | `TransparentHearing` |
| `0x1804` | TransparentHearing_Status_Set | UINT8 status | да | — | `TransparentHearing_Status` |
| `0x1805` | TransparentHearing_Status_Get | — | да | — | `TransparentHearing_Status` |
| `0x1A00` | ANC_SetNoiseCancelationMode | UINT8 mode, UINT8 state | да | — | `ANC` |
| `0x1A01` | ANC_GetNoiseCancelationMode | — | да | — | `ANC` |
| `0x1A02` | ANC_Transparency_Set | UINT8 level | да | — | `ANC_Transparency` |
| `0x1A03` | ANC_Transparency_Get | — | да | — | `ANC_Transparency` |
| `0x1A04` | ANC_SetNoiseCancelation | UINT8 activation | да | — | `ANC_Status` |
| `0x1A05` | ANC_GetNoiseCancelation | — | да | — | `ANC_Status` |

## Vendor 0x001D (Qualcomm core)

| ID | Команда | Параметры запроса | Ответ | Уведомление | Свойство |
|---|---|---|---|---|---|
| `0x0000` | Core_Api_Version_Get | UINT8 otaCommand | да | — | `Core_Api_Version` |
| `0x0001` | Core_GetSupportedFeatures_Get | — | да | — | `Core_GetSupportedFeatures` |
| `0x0002` | Core_GetSupportedFeaturesNext_Get | — | да | — | `Core_GetSupportedFeaturesNext` |
| `0x0003` | Core_GetSerialNumber | — | да | — | `Core_SerialNumber` |
| `0x0007` | Config_RegisterNotification | UINT8 featureID | да | — | `RegisterNotification_qDFU` |
| `0x0009` | Core_DataTransferSetup | UINT16 sessionId, UINT8 transportType | да | — | `Core_DataTransferSetup` |
| `0x000A` | Core_DataTransferGet | UINT16 sessionId, UINT32 startOffset, UINT32 dataSize | да | — | `Core_DataTransferGet` |
| `0x000C` | Core_GetTransportInfo | UINT8 transportKey | да | — | `Core_GetTransportInfo` |
| `0x000D` | Core_SetTransportParameter_Set | UINT8 transportKey, UINT32 value | да | — | `Core_SetTransportParameter` |
| `0x0600` | VoiceUI_Selected_Assistant_Get | — | да | — | `VoiceUI_Selected_Assistant` |
| `0x0601` | VoiceUI_Selected_Assistant_Set | UINT8 provider | да | — | `VoiceUI_Selected_Assistant` |
| `0x0602` | VoiceUI_Supported_Assistants_Get | — | да | — | `VoiceUI_Supported_Assistants` |
| `0x0803` | Debug_GetPanicLogInfo | — | да | — | `Debug_GetPanicLogInfo` |
| `0x0804` | Debug_ErasePanicLog | UINT16 payloadWord | да | — | `Debug_ErasePanicLog` |
| `0x0C00` | UPGRADE_GaiaConnect | — | да | — | `Upgrade_GaiaConnect` |
| `0x0C01` | UPGRADE_GaiaDisconnect | — | да | — | `Upgrade_GaiaDisconnect` |
| `0x0C02` | UPGRADE_VM_UpgradeControl | UINT8 parameter1 | да | — | `Upgrade_TransferComplete_Trigger` |
| `0x1800` | Statistics_GetAvailableCategories | UINT16 Last received ID | да | — | `Statistics_GetAvailableCategories` |
| `0x1801` | Statistics_GetAllStatisticsInCategory | UINT16 Category ID, UINT8 Last received statistics ID | да | — | `Statistics_GetAllStatisticsInCategory` |
| `0x1802` | Statistics_GetStatisticsValues | UINT16 Category ID, UINT8 statistics ID | да | — | `Statistics_GetStatisticsValues` |

